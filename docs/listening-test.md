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

## Part 5: Other languages

Select one paragraph at a time and read it with a voice of its language: `ef_dora` (Spanish), `ff_siwis` (French), `if_sara` (Italian), `pf_dora` (Brazilian Portuguese), `hf_alpha` (Hindi). These voices need espeak-ng installed.

El tren salió de Madrid a las 10:30 y llegó a Sevilla con veinte minutos de retraso. ¿Quién lo habría dicho? En el vagón, el 3,5 % de los viajeros dormía; los demás leían, hablaban o miraban por la ventana. ¡Qué viaje tan largo!

Les enfants sont dans le jardin et jouent sous les grands arbres. Le train de 8 heures est parti à l'heure, mais il pleuvait déjà à Lyon. « Tu viens ce soir ? » m'a demandé Hélène ; je lui ai répondu que oui.

Il treno per Roma parte alle 10:30 dal binario 3. Ciao, come stai? Bene, grazie: oggi fa caldo, ma domani pioverà. Gli gnocchi della nonna sono i migliori.

Não sei se vou conseguir chegar às oito. O ônibus estava cheio, e o trânsito, como sempre, parado. Você quer café ou chá? Obrigado pela atenção!

आज मौसम बहुत अच्छा है। कल शायद बारिश होगी, इसलिए छाता ज़रूर ले जाना। मैं iPhone use करता हूँ। मेरे पास 25 रुपये हैं।

You should hear each paragraph in its own language, with nothing spelled letter by letter: the times as "diez treinta", "huit heures" and "dieci e trenta"; "tres coma cinco por ciento"; the questions rising; a pause at each comma and a full stop at each `।`; and "iPhone use" in English inside the Hindi sentence. With espeak-ng uninstalled, these voices show a notice instead of speaking.

## Part 6: Japanese

Select one paragraph at a time, with a Japanese voice such as `jf_alpha` or `jm_kumo`; first without the Japanese dictionary (Settings → Models), then with it.

きょうも、いいてんきですね。あさごはんをたべてから、こうえんまでさんぽにいきました。ベンチにすわって、コーヒーをのみながら、ちょっとだけほんをよみました。ラジオからきこえてくるおんがくが、とてもきもちよかったです。あしたもまた、いっしょにいきませんか？

私は毎朝、駅前の店でコーヒーを買います。今日は天気がいいので、3人で公園へ行きました。昨日、新しいiPhoneで写真を撮りました。

You should hear the kana paragraph as five Japanese sentences, the last one a question: the small っ in ちょっと and いっしょ as a short catch, ー in コーヒー as a long vowel, ん in さんぽ as "m". Without the dictionary its words run together a little and the kanji paragraph shows "Kanji need the dictionary in Settings → Models" instead. With the dictionary, words come apart, きょう and こうえん have long vowels, and the kanji paragraph is read whole: 私は and 今日は with "wa", 公園へ with "e", 3人 as "san nin", and "iPhone" in English.

## Part 7: Mandarin

Select only this part's paragraph, with a Chinese voice such as `zf_xiaobei` or `zm_yunjian`.

今天天气很好，我们一起去公园散步吧。路上，我在银行门口遇到了一位老朋友，他说他重新开始学画画儿了。我们聊了一会儿，觉得时间过得真快：上次见面还是2019年！你明天有空吗？

You should hear five Mandarin sentences, the last one a question, with the four tones distinct: 今天 level, 银 rising, 好 dipping, 在 falling. 银行 is "yínháng" and 重新 "chóngxīn"; 2019年 is read digit by digit, "èr líng yī jiǔ nián"; 了, the 得 of 觉得 and 吗 are light and unstressed.
