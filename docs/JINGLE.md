# Baukit jingle

This file owns every sung and spoken word in the Baukit video. Preview the requests with `python3 scripts/generate-jingle.py --dry-run`; the regeneration commands below produce the audio when you have an ElevenLabs key.

## Concept

For developers and tech leads who have written the same login five times. One command generates a product with a Rust backend plus web and mobile clients, and `baukit doctor` checks the project. You can write your product code instead of another login handler.

The song is a garage punk-rock construction anthem in E minor at 150 BPM, in 4/4. Crunchy power chords, picked bass, hammer-and-anvil percussion, and hand claps carry a rough older male lead and raw gang vocals. The English hook is "Baukit, bang it together!" The German crowd answers "Baukit, komm, bau mit!" Bill is the trustworthy older foreman for the spoken bookends in both languages. The music model casts its own singers; the voice ID controls only the voiceover. The running joke is the fifth auth rewrite, with a drill standing in for the keyboard.

| Feature in the song | Implemented behavior and source |
| --- | --- |
| CLI product generation | [CLI commands](../cli/src/main.rs) generate a Rust backend, web and mobile clients, with optional auth and a worker. |
| Project checks | [Doctor](../cli/README.md) checks project configuration, dependency wiring, and declared contracts. |
| Prewired login | [Auth generation](../cli/src/lib.rs) wires the selected auth provider into the backend and clients. |
| Safe client retries | [API runtime](../typescript/packages/api-runtime/src/index.ts) bounds retries and respects request safety. |
| Durable background jobs | [Job store](../rust/crates/baukit-jobs/src/store.rs) provides a PostgreSQL outbox for workers. |
| Signed messages between apps | [Suite protocol](platform/suite-events.md) links apps and verifies signed events. Products adopt this component separately; generation does not enable suite linking. |

## Lyrics

Section names are request identifiers shared by both languages. Curly braces mark performance cues. At 150 BPM each bar lasts 1.6 seconds. The short cut has 28 bars, the long cut 48. Each sung line has at least two bars; sections under eight seconds have one line. Leave room for the claps and gang responses.

### English short

```lyrics en short
[Intro]
One command. Your next app starts here!
[Verse]
Rust backend, web in your hand,
Mobile too, from one command.
Run baukit doctor, check the frame,
Fifth auth rewrite? Skip that game!
[Chorus]
Baukit, bang it together!
Auth's wired. Build through bad weather!
Safe retries when wires get crossed,
Queued-up jobs don't just get lost.
Baukit, bang it together!
Auth's wired. Build through bad weather!
[Outro]
{hammer hit}
Grab the kit. Try baukit new!
```

### English long

```lyrics en long
[Intro]
One command. Your next app starts here!
[Verse 1]
Rust backend, web in your hand,
Mobile too, from one command!
Fifth auth rewrite? Step away.
Login's wired. Go out and play!
[Chorus]
Baukit, bang it together!
Auth's wired. Build through bad weather!
Safe retries when wires get crossed,
Queued-up jobs don't just get lost.
[Verse 2]
baukit doctor checks the frame,
Loose screws get flagged by name.
Link two apps. Signed notes go through,
Both check who sent it to you.
[Chorus 2]
Baukit, bang it together!
Auth's wired. Build through bad weather!
Safe retries when wires get crossed,
Queued-up jobs don't just get lost.
[Bridge]
{stop for hand claps}
Fifth auth rewrite? Put down that drill!
[Final chorus]
Baukit, bang it together!
Auth's wired. Build through bad weather!
Name your app. There's work to do!
Grab the kit. Try baukit new!
```

### German short

```lyrics de short
[Intro]
Ein Befehl. Dein Projekt geht los!
[Verse]
Rust im Rücken, Web am Start,
Handy-App, viel Zeit gespart.
baukit doctor prüft den Plan,
Login hängt am Kabel dran!
[Chorus]
Baukit, komm, bau mit! (hey, hey)
Ein Befehl, dein nächster Schritt!
Retries, wenn die Leitung kracht,
Jobs gespeichert. Gute Nacht!
Baukit, komm, bau mit! (hey, hey)
Ein Befehl, dein nächster Schritt!
[Outro]
{hammer hit}
Tipp baukit new. Bau was draus!
```

### German long

```lyrics de long
[Intro]
Ein Befehl. Dein Projekt geht los!
[Verse 1]
Rust im Rücken, Web am Start,
Handy-App, viel Zeit gespart.
Fünfmal Auth? Du lässt das sein,
Login baut dir Baukit ein!
[Chorus]
Baukit, komm, bau mit! (hey, hey)
Ein Befehl, dein nächster Schritt!
Retries, wenn die Leitung kracht,
Jobs gespeichert. Gute Nacht!
[Verse 2]
baukit doctor prüft den Bau,
Zeigt dir Fehler ganz genau.
Zwei Apps, Post mit Unterschrift,
Beide prüfen, was eintrifft.
[Chorus 2]
Baukit, komm, bau mit! (hey, hey)
Ein Befehl, dein nächster Schritt!
Retries, wenn die Leitung kracht,
Jobs gespeichert. Gute Nacht!
[Bridge]
{stop for hand claps}
Fünfmal Auth? Leg den Bohrer weg!
[Final chorus]
Baukit, komm, bau mit! (hey, hey)
Ein Befehl, dein nächster Schritt!
Hol den Bausatz, pack ihn aus,
Tipp baukit new. Bau was draus!
```

## Voiceover

### English intro

```voiceover en intro
[curious] Writing auth for the fifth time? [laughs] Put the keyboard down a second. We've got a kit for that.
```

### English outro

```voiceover en outro
[excited] Baukit. Bang it together. Generate your next Rust backend, web app, and mobile app with baukit new. Try the kit.
```

### German intro

```voiceover de intro
[curious] Schreibst du zum fünften Mal Auth? [laughs] Lass mal die Tastatur los. Dafür gibt's einen Bausatz.
```

### German outro

```voiceover de outro
[excited] Baukit. Komm, bau mit. Tipp baukit new für dein nächstes Projekt mit Rust-Backend, Web-App und Handy-App. Probier den Bausatz aus.
```

## Video storyboard

Times are relative to the start of the music in either language. Put the intro voiceover before 0.0 and the outro voiceover after the music ends; their generated lengths are separate. Sound effects start at the beginning of the indicated section. An effect appears only where its ID is listed; "none" means no added effect. These are edit timings based on the requested chunk lengths. Check the rendered audio before locking the video cut.

### Short version

| Section | Start (s) | End (s) | On screen | Sound effect |
| --- | --- | --- | --- | --- |
| Intro | 0.0 | 6.4 | Run baukit new orders --backend --web --mobile --auth oidc --worker; a product file tree fills the workbench. | anvil |
| Verse | 6.4 | 19.2 | Show the backend, web and mobile clients running after the generated startup steps. Cut to baukit doctor findings and the wired login. | blueprint |
| Chorus | 19.2 | 38.4 | Show a safe GET retry and a persisted PostgreSQL job. Clap cuts return to the developer's own product code. | relay |
| Outro | 38.4 | 44.8 | Full-screen generation command with a product name and --backend --web --mobile, then the repository URL. | anvil |

### Long version

| Section | Start (s) | End (s) | On screen | Sound effect |
| --- | --- | --- | --- | --- |
| Intro | 0.0 | 6.4 | Run baukit new orders --backend --web --mobile --auth oidc --worker. Reveal the generated product. | anvil |
| Verse 1 | 6.4 | 19.2 | Show the Rust backend, web and mobile clients running after startup. Drop "Auth rewrite #5" in the bin and show login. | blueprint |
| Chorus | 19.2 | 32.0 | A safe GET retry reconnects a cable; a PostgreSQL job stays in its outbox while the developer leaves the bench. | relay |
| Verse 2 | 32.0 | 44.8 | Show baukit doctor naming a broken setting, then an adopted suite link sending and verifying a signed event between two apps. | none |
| Chorus 2 | 44.8 | 57.6 | Repeat retry and job shots, then show the worker claiming the stored job. | none |
| Bridge | 57.6 | 64.0 | The foreman sets down a drill on the hand claps. The developer closes the fifth auth ticket. | none |
| Final chorus | 64.0 | 76.8 | Show the generation command with a product name and --backend --web --mobile. End on github.com/PatrickKoss/baukit. | anvil |

## Sound design

- `anvil`: a compact hammer-on-anvil logo sting at the intro and final tag of either cut.
- `blueprint`: a paper blueprint snap with a short socket-wrench click at the short verse and long first verse.
- `relay`: a mechanical relay stepping into a firm latch, under retries and the stored job in the first chorus of either cut.

Prompts live in `SOUND_EFFECTS` in the script. Mix the effects below the lyrics; the script exports separate files so the editor controls their placement and level.

## Regenerate

Run from the repository root. Python 3 and its standard library are enough. The dry run checks lyrics against the section plan, validates voiceovers and storyboard timings, then saves the exact request bodies without contacting ElevenLabs:

```bash
python3 scripts/generate-jingle.py --dry-run
```

For real generation, read the key without storing it in shell history, then export it. This command can incur ElevenLabs charges:

```bash
read -rs ELEVENLABS_API_KEY
export ELEVENLABS_API_KEY
python3 scripts/generate-jingle.py
unset ELEVENLABS_API_KEY
```

The CLI also accepts `--api-key KEY`, but the environment variable keeps the key out of the command line. Select individual outputs, change models, or choose another destination:

```bash
python3 scripts/generate-jingle.py --dry-run --language en --version short --parts music
python3 scripts/generate-jingle.py --dry-run --language de --version long --parts music,voice
python3 scripts/generate-jingle.py --dry-run --parts sfx
python3 scripts/generate-jingle.py --dry-run --parts voice --tts-model eleven_v3
python3 scripts/generate-jingle.py --dry-run --music-model music_v2_5 --out-dir out/jingle-review
python3 scripts/generate-jingle.py --dry-run --language both --version both --force
```

Remove `--dry-run` for the chosen real run after setting the key. `--version` selects music only. `--language` selects music and voiceovers; effects are generated once. Defaults select both languages, both cuts, and all parts. `--force` regenerates existing audio and spends credits again. Without it, audio files are skipped even if the document or model has changed. There are no automatic paid retries.

The default destination is `out/jingle/`, resolved against the repository root even when invoked elsewhere. An explicit relative `--out-dir` starts at your current working directory. Music files are `baukit-jingle-<short|long>-<en|de>.mp3`; voiceovers are `baukit-voiceover-<intro|outro>-<en|de>.mp3`; effects are `baukit-sfx-<id>.mp3`. Dry runs write matching `*.request.json` files containing only the JSON bodies. `manifest.json` describes audio files, with endpoints, bodies, models, seeds, document hashes, response headers, and audio hashes and sizes. Dry runs preserve its audio records and add no entries for planned audio. Skipped files keep their existing entries unchanged.

All selected voices are checked before any paid request. The v4 speech model receives stability and similarity settings; its pacing comes from punctuation and audio tags. The v3 override receives the full voice preset. Fixed seeds and saved bodies make the inputs repeatable; ElevenLabs may change model behavior, so identical audio bytes are not guaranteed. The document SHA-256 identifies the copy used for each generation.
