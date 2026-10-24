# Mouthy listening test for text reading

Open this file in a text editor, not a Markdown preview, so its line breaks stay as written. Select the whole file or one part, press the Mouthy hotkey and listen. Each part ends with what you should hear, which is read aloud too. Use the offline voice: most of this is about how it reads.

## Part 1: Initialisms and names

Your ID is ready to use now.

The API URL is in the README.

Open Settings on macOS or iOS.

In the US I learned to drive. Show your US ID card.

We use JSON and YAML, NASA style.

Install Node.js and GitHub Desktop.

[TODO: fix the API before release]

Prices rose in the U.S. Analysts expect more.

The IDs and URLs are listed below.

You should hear: ID, API, URL, US and IDs spelled letter by letter; README as "read me"; macOS as "mac O S" and iOS as "eye O S"; JSON as "Jason" and YAML as "yam el"; NASA as one word; Node.js as "Node dot J S" and GitHub as "Git Hub". In the bracketed line every word is spoken, none dropped before "API". The line about prices is two sentences, with a full stop after the U.S. letters.

## Part 2: Wrapped text

Mouthy reads text copied from Safari, Mail, Notes and
Visual Studio Code aloud, and I use it every day with
Claude Code to hear long replies.

- The offline voice reads the selection when you press the
  hotkey, and stops when you press the hotkey again.
- Nothing you select leaves your Mac, because the voice runs
  on your own computer once its files are downloaded.

Getting started with Mouthy
Choosing a voice for reading
Reading a selection aloud

alice: did the new build fix the pauses
bob: yes, wrapped lines read as one sentence now
alice: great, I will try it after lunch

Voice name	Accent	Reading speed
Alloy	American	normal
Lewis	British	faster

INFO Starting server on port 8080 with 4 workers ok
WARN Disk usage on the main volume is at 91 percent
ERROR Failed to connect to the database at db:5432

E0308 mismatched types in src/tts/text.rs line 12
E0425 cannot find value foo in this scope here

⏺ The splitter treats every line break as the end of a
  sentence, so each wrapped line of a terminal selection
  gets its own pause. That rule exists so headings and
  bullets without punctuation still read as sentences.

⏺ Bash(git log --oneline -3)
  ⎿  a1b2c3d fix: reflow wrapped lines before sentence splitting
     9f8e7d6 feat: default voice alloy for fresh installs
     e4f5a6b chore: bump version to 0.2.0 for the release

```
let words = text.split_whitespace();
println!("{}", words.count());
```

You should hear: the Safari paragraph as one sentence, with no stop before "Visual Studio Code" or "Claude Code"; each bullet as one sentence; the three headings, the three chat lines, the three table rows, the three log lines and the two compiler lines each as their own sentence; the Claude Code paragraph as two whole sentences; the command and each commit line on its own. The code block is read as written, one sentence per line; its fences are silent.

## Part 3: Pauses and sentences

When the committee met again in the spring, after a long winter of arguments about the budget, the members agreed, with some reluctance, that the old library would stay open on Saturdays, that the reading room would be repainted before the summer, and that the money left over would go to the children's books that had been promised for years.

It works. Try it now.

Why the voice pauses

Each sentence is read in one go, so its tone rises and falls as one sentence and pauses only where the commas are.

Dr. Smith met Mr. Jones at 7.30pm.

Read pages 10-12 first.

You should hear: the committee sentence in one tone, with short pauses at its commas and no longer gap (selected alone, it starts after its first comma, so expect a seam there); a full pause after "It works"; the heading, a full pause, then its paragraph; "Doctor Smith met Mister Jones at seven thirty P M", with no pause after "Doctor" or "Mister"; and "pages ten to twelve".

## Part 4: Dots and numbers

Visit example.com and open settings.rs.

Update to v2.0 today.

The router is at 192.168.0.1.

The file is 3.5 GB.

It costs £4.99.

It was popular in the 1990s.

You should hear: "example dot com" and "settings dot R S"; "version two point zero"; the address as "one hundred and ninety-two point one hundred and sixty-eight point zero point one"; "three point five G B"; "four pounds and ninety-nine pence"; "the nineteen nineties".
