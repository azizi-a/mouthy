//! The dictionary Japanese voices read kanji with: NAIST-jdic as jpreprocess
//! builds it (BSD-3-Clause), downloaded from the Models page as one `.tar.gz`
//! and unpacked beside the Kokoro files.

use super::download;
use flate2::read::GzDecoder;
use jpreprocess::{DefaultTokenizer, Dictionary, JPreprocess};
use log::{info, warn};
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Splits Japanese text into words with their pronunciations.
pub type Tagger = JPreprocess<DefaultTokenizer>;

/// The folder the archive keeps the files in, and the one they go to.
const FOLDER: &str = "naist-jdic";

/// Every file of the dictionary, with its size.
const FILES: &[(&str, u64)] = &[
    ("char_def.bin", 2_356),
    ("dict.da", 33_049_069),
    ("dict.vals", 7_889_010),
    ("dict.words", 32_164_203),
    ("dict.wordsidx", 3_155_608),
    ("matrix.mtx", 3_792_264),
    ("metadata.json", 767),
    ("unk.bin", 2_492),
];

const BLOCK: usize = 512;

/// Room in the archive beyond the files themselves, for a header per entry,
/// padding and the end blocks. Reading stops there, so a damaged archive
/// cannot fill the disk.
const TAR_OVERHEAD: u64 = 64 * BLOCK as u64;

pub fn dir(models_dir: &Path) -> PathBuf {
    models_dir.join(FOLDER)
}

/// Where the download lands before it is unpacked.
pub fn archive(models_dir: &Path) -> PathBuf {
    models_dir.join(format!("{FOLDER}.tar.gz"))
}

fn unpacking(models_dir: &Path) -> PathBuf {
    models_dir.join(format!("{FOLDER}.unpacking"))
}

/// Whether every file is there at its size. The archive was checked against
/// its sha256 before it was unpacked, so its contents are not hashed again.
pub fn installed(dir: &Path) -> bool {
    complete(dir, FILES)
}

fn complete(dir: &Path, files: &[(&str, u64)]) -> bool {
    files.iter().all(|(name, size)| {
        fs::metadata(dir.join(name)).is_ok_and(|meta| meta.is_file() && meta.len() == *size)
    })
}

/// The file whose stamp tells one download of the dictionary from the next.
pub fn stamp_path(dir: &Path) -> PathBuf {
    dir.join("dict.da")
}

/// Loads the tagger. The files are mapped rather than read, so only the parts
/// a text needs come into memory.
pub fn load(dir: &Path) -> Result<Tagger, String> {
    let started = Instant::now();
    let dictionary = Dictionary::load_from_path_with_options(dir, true)
        .map_err(|e| format!("Cannot load the Japanese dictionary: {e}"))?;
    info!(
        "Loaded the Japanese dictionary in {} ms",
        started.elapsed().as_millis()
    );
    Ok(JPreprocess::with_dictionaries(dictionary, None))
}

/// Unpacks the checked download in `models_dir`, replacing any dictionary
/// there, then deletes the download.
pub fn unpack(models_dir: &Path) -> Result<(), String> {
    unpack_files(
        &archive(models_dir),
        &unpacking(models_dir),
        &dir(models_dir),
        FILES,
    )
}

/// Deletes the dictionary and anything an interrupted unpack left. A session
/// reading Japanese keeps its mapped copy until the voice reloads.
pub fn remove(models_dir: &Path) -> Result<(), String> {
    remove_leftovers(models_dir)?;
    remove_dir(&dir(models_dir))
}

/// Deletes what only an unpack in progress should have: the downloaded
/// archive and the folder it is unpacked into.
pub fn remove_leftovers(models_dir: &Path) -> Result<(), String> {
    download::remove_if_present(&archive(models_dir))?;
    remove_dir(&unpacking(models_dir))
}

/// Writes `files` from `archive` into `temp`, then renames it to `dest`, so
/// `dest` holds the whole dictionary or none of it.
fn unpack_files(
    archive: &Path,
    temp: &Path,
    dest: &Path,
    files: &[(&str, u64)],
) -> Result<(), String> {
    let result = File::open(archive)
        .map_err(|e| format!("Cannot read the downloaded dictionary: {e}"))
        .and_then(|file| extract(GzDecoder::new(BufReader::new(file)), temp, files))
        .and_then(|()| {
            remove_dir(dest)?;
            fs::rename(temp, dest)
                .map_err(|e| format!("Cannot move the dictionary into place: {e}"))
        });
    if result.is_err() {
        // Best effort: the next unpack clears it too.
        let _ = remove_dir(temp);
    }
    // Checked before unpacking, so a failure here is the disk's; the next
    // attempt downloads it afresh either way.
    if let Err(e) = download::remove_if_present(archive) {
        warn!("{e}");
    }
    result
}

/// Writes the tar stream's `files` into a fresh `dest`. Anything but the
/// dictionary folder and those files, as regular files at their listed sizes,
/// fails the whole unpack.
fn extract(stream: impl Read, dest: &Path, files: &[(&str, u64)]) -> Result<(), String> {
    remove_dir(dest)?;
    // Fails if anything took the name since: the folder must be our own.
    fs::create_dir(dest)
        .map_err(|e| format!("Cannot create the folder {}: {e}", dest.display()))?;
    let limit = files.iter().map(|(_, size)| size).sum::<u64>() + TAR_OVERHEAD;
    let mut tar = stream.take(limit);
    let mut written = HashSet::new();
    loop {
        let mut block = [0u8; BLOCK];
        tar.read_exact(&mut block).map_err(damaged)?;
        // The end. What follows, and the gzip trailer, go unchecked: the
        // archive's sha256 was checked before it was unpacked.
        if block.iter().all(|&b| b == 0) {
            break;
        }
        let entry = Entry::parse(&block)?;
        let file = entry
            .name
            .strip_prefix(FOLDER)
            .and_then(|rest| rest.strip_prefix('/'));
        match (entry.kind, file) {
            (b'5', Some("")) if entry.size == 0 => {}
            (b'0' | 0, Some(name)) => {
                let &(name, size) = files
                    .iter()
                    .find(|(known, _)| *known == name)
                    .ok_or_else(|| unexpected(&entry.name))?;
                if entry.size != size || !written.insert(name) {
                    return Err(unexpected(&entry.name));
                }
                write_file(&mut tar, &dest.join(name), size)?;
                let padding = (BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64;
                skip(&mut tar, padding)?;
            }
            _ => return Err(unexpected(&entry.name)),
        }
    }
    if written.len() != files.len() {
        return Err("The downloaded dictionary is incomplete".into());
    }
    Ok(())
}

/// One tar header: the entry's path, type and size.
struct Entry {
    name: String,
    kind: u8,
    size: u64,
}

impl Entry {
    fn parse(block: &[u8; BLOCK]) -> Result<Self, String> {
        let damaged = || damaged("a header does not check out");
        let stored = octal(&block[148..156]).ok_or_else(damaged)?;
        let sum: u64 = block
            .iter()
            .enumerate()
            .map(|(i, &b)| u64::from(if (148..156).contains(&i) { b' ' } else { b }))
            .sum();
        // A POSIX path prefix would make the full name longer than the one read.
        let prefixed = &block[257..263] == b"ustar\0" && block[345] != 0;
        if stored != sum || prefixed {
            return Err(damaged());
        }
        let name = &block[..100];
        let name = &name[..name.iter().position(|&b| b == 0).unwrap_or(name.len())];
        Ok(Self {
            name: String::from_utf8_lossy(name).into_owned(),
            kind: block[156],
            size: octal(&block[124..136]).ok_or_else(damaged)?,
        })
    }
}

/// A tar number: octal digits, padded with spaces or NULs.
fn octal(field: &[u8]) -> Option<u64> {
    let digits = std::str::from_utf8(field)
        .ok()?
        .trim_matches(|c: char| c == ' ' || c == '\0');
    if digits.is_empty() {
        return Some(0);
    }
    u64::from_str_radix(digits, 8).ok()
}

fn write_file(tar: &mut impl Read, path: &Path, size: u64) -> Result<(), String> {
    let cannot = |e: io::Error| format!("Cannot write {}: {e}", path.display());
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(cannot)?;
    let copied = io::copy(&mut tar.take(size), &mut file).map_err(cannot)?;
    if copied != size {
        return Err(damaged("it ends early"));
    }
    // On disk before the folder is renamed into place.
    file.sync_all().map_err(cannot)
}

fn skip(tar: &mut impl Read, bytes: u64) -> Result<(), String> {
    let skipped = io::copy(&mut tar.take(bytes), &mut io::sink()).map_err(damaged)?;
    if skipped != bytes {
        return Err(damaged("it ends early"));
    }
    Ok(())
}

fn damaged(detail: impl std::fmt::Display) -> String {
    format!("The downloaded dictionary is damaged: {detail}")
}

fn unexpected(name: &str) -> String {
    format!("The downloaded dictionary holds an unexpected entry: {name:?}")
}

fn remove_dir(path: &Path) -> Result<(), String> {
    match fs::remove_dir_all(path) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => {
            Err(format!("Cannot delete {}: {e}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Files that pass for an installed dictionary, sparse so they cost nothing.
#[cfg(test)]
pub(crate) fn install_placeholder(models_dir: &Path) {
    let dir = dir(models_dir);
    fs::create_dir_all(&dir).unwrap();
    for (name, size) in FILES {
        File::create(dir.join(name))
            .unwrap()
            .set_len(*size)
            .unwrap();
    }
}

/// The dictionary for a test that reads with it: the archive at
/// `MOUTHY_JA_ARCHIVE`, checked against the catalog's sha256, unpacked and
/// loaded as the app does. Without it the test passes having checked nothing,
/// unless `MOUTHY_REQUIRE_JA_DICTIONARY` is set, as on Linux CI.
#[cfg(test)]
pub(crate) fn tagger_for_test() -> Option<&'static Tagger> {
    static TAGGER: std::sync::OnceLock<Option<Tagger>> = std::sync::OnceLock::new();
    let tagger = TAGGER
        .get_or_init(|| {
            let downloaded = PathBuf::from(std::env::var_os("MOUTHY_JA_ARCHIVE")?);
            // Kept between runs, so a run unpacks only once.
            let models = std::env::temp_dir().join("mouthy-test-models");
            if !installed(&dir(&models)) {
                let pinned = super::catalog::all(&models)
                    .into_iter()
                    .find(|file| file.kind == super::catalog::ModelFileKind::Dictionary)
                    .unwrap();
                let actual = download::sha256_file(&downloaded).unwrap();
                assert_eq!(actual, pinned.sha256, "{}", downloaded.display());
                fs::create_dir_all(&models).unwrap();
                fs::copy(&downloaded, archive(&models)).unwrap();
                unpack(&models).unwrap();
            }
            Some(load(&dir(&models)).unwrap())
        })
        .as_ref();
    if tagger.is_none() {
        assert!(
            std::env::var_os("MOUTHY_REQUIRE_JA_DICTIONARY").is_none(),
            "the Japanese dictionary is required here: set MOUTHY_JA_ARCHIVE"
        );
        eprintln!("MOUTHY_JA_ARCHIVE is not set, so this test checks nothing");
    }
    tagger
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    const SMALL: &[(&str, u64)] = &[("a.bin", 5), ("b.bin", 700)];

    /// A tar entry: name, type, declared size, body.
    type TarEntry = (String, u8, u64, Vec<u8>);

    fn header(name: &str, kind: u8, size: u64) -> [u8; BLOCK] {
        let mut block = [0u8; BLOCK];
        block[..name.len()].copy_from_slice(name.as_bytes());
        block[100..107].copy_from_slice(b"0000644");
        block[124..135].copy_from_slice(format!("{size:011o}").as_bytes());
        block[136..147].copy_from_slice(b"00000000000");
        block[156] = kind;
        block[257..265].copy_from_slice(b"ustar  \0");
        block[148..156].copy_from_slice(b"        ");
        let sum: u64 = block.iter().map(|&b| u64::from(b)).sum();
        block[148..155].copy_from_slice(format!("{sum:06o}\0").as_bytes());
        block
    }

    /// The entries as GNU tar writes them.
    fn tar(entries: &[TarEntry]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, kind, size, body) in entries {
            out.extend_from_slice(&header(name, *kind, *size));
            out.extend_from_slice(body);
            out.resize(out.len().div_ceil(BLOCK) * BLOCK, 0);
        }
        out.extend_from_slice(&[0; 2 * BLOCK]);
        out
    }

    fn file(name: &str, size: usize) -> TarEntry {
        (name.to_string(), b'0', size as u64, vec![b'x'; size])
    }

    fn folder(name: &str) -> TarEntry {
        (name.to_string(), b'5', 0, Vec::new())
    }

    fn good() -> Vec<TarEntry> {
        vec![
            folder("naist-jdic/"),
            file("naist-jdic/a.bin", 5),
            file("naist-jdic/b.bin", 700),
        ]
    }

    fn gz(bytes: &[u8]) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    }

    /// Unpacks `download` as if it had just been downloaded into `models`.
    fn unpack_download(models: &Path, download: &[u8]) -> Result<(), String> {
        fs::write(archive(models), download).unwrap();
        unpack_files(&archive(models), &unpacking(models), &dir(models), SMALL)
    }

    /// Nothing is installed and nothing is left half-made.
    fn assert_empty(models: &Path) {
        let left: Vec<_> = fs::read_dir(models)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(left.is_empty(), "{left:?}");
    }

    #[test]
    fn a_whole_archive_unpacks_into_place_and_the_download_goes() {
        let models = tempfile::tempdir().unwrap();
        unpack_download(models.path(), &gz(&tar(&good()))).unwrap();
        let dir = dir(models.path());
        assert!(complete(&dir, SMALL));
        assert_eq!(fs::read(dir.join("a.bin")).unwrap(), b"xxxxx");
        assert!(!archive(models.path()).exists());
        assert!(!unpacking(models.path()).exists());
    }

    #[test]
    fn a_new_dictionary_replaces_the_old_one() {
        let models = tempfile::tempdir().unwrap();
        unpack_download(models.path(), &gz(&tar(&good()))).unwrap();
        fs::write(dir(models.path()).join("stale"), b"old").unwrap();
        // And what an interrupted unpack left goes too.
        fs::create_dir_all(unpacking(models.path())).unwrap();
        fs::write(unpacking(models.path()).join("a.bin"), b"half").unwrap();
        unpack_download(models.path(), &gz(&tar(&good()))).unwrap();
        assert!(complete(&dir(models.path()), SMALL));
        assert!(!dir(models.path()).join("stale").exists());
        assert!(!unpacking(models.path()).exists());
    }

    #[test]
    fn a_dangerous_archive_installs_nothing() {
        let with = |change: &dyn Fn(&mut Vec<TarEntry>)| {
            let mut entries = good();
            change(&mut entries);
            entries
        };
        let link = |kind| ("naist-jdic/a.bin".to_string(), kind, 0, Vec::new());
        for (case, entries) in [
            (
                "a path out of the folder",
                with(&|e| e[1].0 = "naist-jdic/../a.bin".into()),
            ),
            ("a path above it", with(&|e| e.push(file("../a.bin", 5)))),
            ("an absolute path", with(&|e| e.push(file("/tmp/a.bin", 5)))),
            ("a symbolic link", with(&|e| e[1] = link(b'2'))),
            ("a hard link", with(&|e| e[1] = link(b'1'))),
            (
                "another folder",
                with(&|e| e.push(folder("naist-jdic/sub/"))),
            ),
            ("a folder elsewhere", with(&|e| e[0] = folder("other/"))),
            (
                "an extra file",
                with(&|e| e.push(file("naist-jdic/c.bin", 5))),
            ),
            (
                "a file twice",
                with(&|e| e.push(file("naist-jdic/a.bin", 5))),
            ),
            (
                "a file too big",
                with(&|e| e[1] = file("naist-jdic/a.bin", 6)),
            ),
            ("a file missing", with(&|e| e.truncate(2))),
            (
                "a long name entry",
                with(&|e| e.insert(1, ("././@LongLink".into(), b'L', 20, vec![b'a'; 20]))),
            ),
        ] {
            let models = tempfile::tempdir().unwrap();
            let result = unpack_download(models.path(), &gz(&tar(&entries)));
            assert!(result.is_err(), "{case}");
            assert_empty(models.path());
        }
    }

    #[test]
    fn a_truncated_or_corrupt_archive_installs_nothing() {
        let whole = tar(&good());
        let mut bad_sum = whole.clone();
        bad_sum[BLOCK + 10] ^= 1;
        let mut cut_gzip = gz(&whole);
        cut_gzip.truncate(cut_gzip.len() / 2);
        for (case, download) in [
            ("cut mid-file", gz(&whole[..BLOCK * 2 + 3])),
            ("no end blocks", gz(&whole[..whole.len() - 2 * BLOCK])),
            ("a bad checksum", gz(&bad_sum)),
            ("a cut gzip stream", cut_gzip),
            ("not gzip", whole.clone()),
        ] {
            let models = tempfile::tempdir().unwrap();
            assert!(unpack_download(models.path(), &download).is_err(), "{case}");
            assert_empty(models.path());
        }
    }

    #[test]
    fn an_endless_archive_stops_at_the_size_limit() {
        // Folder entries are allowed, so only the byte limit stops these.
        let mut entries = good();
        entries.splice(1..1, (0..100).map(|_| folder("naist-jdic/")));
        let models = tempfile::tempdir().unwrap();
        let error = unpack_download(models.path(), &gz(&tar(&entries))).unwrap_err();
        assert!(error.contains("damaged"), "{error}");
        assert_empty(models.path());
    }

    #[test]
    fn installed_means_every_file_at_its_size() {
        let models = tempfile::tempdir().unwrap();
        let dir = dir(models.path());
        assert!(!complete(&dir, SMALL));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.bin"), b"xxxxx").unwrap();
        assert!(!complete(&dir, SMALL));
        fs::write(dir.join("b.bin"), vec![b'x'; 699]).unwrap();
        assert!(!complete(&dir, SMALL));
        fs::write(dir.join("b.bin"), vec![b'x'; 700]).unwrap();
        assert!(complete(&dir, SMALL));
        remove(models.path()).unwrap();
        assert!(!dir.exists());
    }

    #[test]
    fn leftovers_go_and_the_dictionary_stays() {
        let models = tempfile::tempdir().unwrap();
        unpack_download(models.path(), &gz(&tar(&good()))).unwrap();
        fs::write(archive(models.path()), b"stale").unwrap();
        fs::create_dir_all(unpacking(models.path())).unwrap();
        fs::write(unpacking(models.path()).join("a.bin"), b"half").unwrap();
        remove_leftovers(models.path()).unwrap();
        assert!(!archive(models.path()).exists());
        assert!(!unpacking(models.path()).exists());
        assert!(complete(&dir(models.path()), SMALL));
        // Nothing left is no error.
        remove_leftovers(models.path()).unwrap();
    }

    #[test]
    fn the_files_are_the_published_ones() {
        let total: u64 = FILES.iter().map(|(_, size)| size).sum();
        assert_eq!(total, 80_055_769);
        assert_eq!(archive(Path::new("/m")), Path::new("/m/naist-jdic.tar.gz"));
        assert_eq!(dir(Path::new("/m")), Path::new("/m/naist-jdic"));
    }

    #[test]
    fn the_published_archive_unpacks_whole_and_loads() {
        if tagger_for_test().is_none() {
            return;
        }
        let models = tempfile::tempdir().unwrap();
        let downloaded = std::env::var_os("MOUTHY_JA_ARCHIVE").unwrap();
        fs::copy(downloaded, archive(models.path())).unwrap();
        unpack(models.path()).unwrap();
        assert!(installed(&dir(models.path())));
        assert!(!archive(models.path()).exists());
        assert!(!unpacking(models.path()).exists());
        let tagger = load(&dir(models.path())).unwrap();
        let nodes = tagger.text_to_njd("東京").unwrap().nodes;
        assert_eq!(nodes[0].get_pron().to_string(), "トーキョー");
    }
}
