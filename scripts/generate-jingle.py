#!/usr/bin/env python3
"""Generate and mix Baukit music, voiceovers, and video effects from docs/JINGLE.md."""

from __future__ import annotations

import argparse
import hashlib
import http.client
import json
import os
import re
import shutil
import subprocess
import sys
import urllib.error
import urllib.request
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DOC = ROOT / "docs/JINGLE.md"
APP_SLUG = "baukit"
API_BASE = "https://api.elevenlabs.io"
LANGUAGES = ("en", "de")
VERSIONS = ("short", "long")
PARTS = ("music", "voice", "sfx", "mix")
PAID_PARTS = ("music", "voice", "sfx")
TTS_MODEL = "eleven_v4"
TTS_OUTPUT_FORMAT = "mp3_44100_128"
TTS_SEED = 2050150
MUSIC_TIMEOUT_S = 600
REQUEST_TIMEOUT_S = 120
SFX_MODEL = "eleven_text_to_sound_v2"
SFX_OUTPUT_FORMAT = "mp3_44100_128"
LYRIC_LINE_LIMIT = 200
SECTION_LINE_LIMIT = 30
VOICEOVER_CHARACTER_LIMIT = 400
MAX_CHUNKS = 30
CHUNK_DURATION_RANGE = (3000, 120000)
VERSION_DURATION_RANGES = {"short": (42000, 48000), "long": (60000, 90000)}
SFX_DURATION_RANGE = (0.5, 30)
SFX_INFLUENCE_RANGE = (0, 1)
MUSIC_BPM = 150
BEATS_PER_BAR = 4
RESPONSE_HEADERS = ("song-id", "request-id", "character-cost")
MIX_GAP_SECONDS = 0.3
MIX_SAMPLE_RATE = 44100
MIX_BITRATE = "192k"
MIX_SFX_GAIN = 0.7
MIX_LIMIT = 0.89
MIX_LOUDNESS_LUFS = -14
MIX_TRUE_PEAK_DB = -1.5
MIX_LOUDNESS_RANGE = 11
FFMPEG_TIMEOUT_S = 300
FFMPEG_MISSING = "ffmpeg is required to mix; install it or leave mix out of --parts"
BILL_VOICE = {
    "voice_id": "pqHfZKP75CvOlQylNhV4",
    "name": "Bill",
    "voice_settings": {
        "stability": 0.5,
        "similarity_boost": 0.75,
        "style": 0.3,
        "use_speaker_boost": True,
        "speed": 1.0,
    },
}
VOICES = dict.fromkeys(LANGUAGES, BILL_VOICE)
INTRO_CHUNK = {
    "section": "Intro",
    "duration_ms": 6400,
    "positive_styles": (
        "garage punk rock", f"{MUSIC_BPM} BPM", "E minor", "crunchy power chord guitars",
        "picked electric bass", "hammer and anvil percussion", "hand claps",
        "raw older male lead and gang vocals", "playful defiance", "dry live room production",
    ),
    "negative_styles": ("polished corporate pop", "slow ballad", "fade out"),
}
VERSE_STYLES = {
    "positive_styles": ("punchy rhythmic vocals", "palm muted power chords"),
    "negative_styles": ("guitar solo",),
}
CHORUS_STYLES = {
    "positive_styles": ("unison gang hook", "open power chords", "hand claps"),
    "negative_styles": ("soft vocals",),
}
MUSIC = {
    "model_id": "music_v2_5",
    "output_format": "mp3_48000_192",
    "short": {
        "seed": 150448,
        "chunks": (
            INTRO_CHUNK,
            {"section": "Verse", "duration_ms": 12800, **VERSE_STYLES},
            {"section": "Chorus", "duration_ms": 19200, **CHORUS_STYLES},
            {"section": "Outro", "duration_ms": 6400,
             "positive_styles": ("shouted gang vocals", "metallic anvil accents", "hard stop"),
             "negative_styles": ("fade out",)},
        ),
    },
    "long": {
        "seed": 150768,
        "chunks": (
            INTRO_CHUNK,
            {"section": "Verse 1", "duration_ms": 12800, **VERSE_STYLES},
            {"section": "Chorus", "duration_ms": 12800, **CHORUS_STYLES},
            {"section": "Verse 2", "duration_ms": 12800,
             "positive_styles": ("driving bass", "hammer percussion", "rhythmic lead vocal"),
             "negative_styles": ("tempo change",)},
            {"section": "Chorus 2", "duration_ms": 12800, **CHORUS_STYLES},
            {"section": "Bridge", "duration_ms": 6400,
             "positive_styles": ("stop time hand claps", "wry shouted lead"),
             "negative_styles": ("long instrumental solo",)},
            {"section": "Final chorus", "duration_ms": 12800,
             "positive_styles": ("full gang vocals", "hard stop"),
             "negative_styles": ("fade out",)},
        ),
    },
}
SOUND_EFFECTS = {
    "anvil": {
        "text": ("One heavy workshop hammer striking a steel anvil, a tight metallic ring "
                 "and short room decay, compact logo punctuation, no voices or music."),
        "duration_seconds": 1.2,
        "prompt_influence": 0.8,
    },
    "blueprint": {
        "text": ("A stiff paper blueprint snapping open on a wooden workbench, followed by "
                 "one crisp socket wrench click, close dry recording, no voices or music."),
        "duration_seconds": 0.8,
        "prompt_influence": 0.8,
    },
    "relay": {
        "text": ("Three mechanical relay clicks stepping evenly into one firm metal latch, "
                 "like a workshop job counter completing a task, no voices or music."),
        "duration_seconds": 2.0,
        "prompt_influence": 0.75,
    },
}


@dataclass(frozen=True)
class Job:
    filename: str
    endpoint: str
    body: dict
    timeout: int
    voice_id: str | None = None


def parse_document(text: str) -> dict:
    blocks = {}
    active = None
    lines = []
    for line in text.splitlines():
        if active is not None:
            if line == "```":
                if active and active[0] in ("lyrics", "voiceover"):
                    if active in blocks:
                        raise ValueError(f"duplicate block: {' '.join(active)}")
                    blocks[active] = "\n".join(lines)
                active = None
                lines = []
            else:
                lines.append(line)
            continue
        if line.startswith("```"):
            active = tuple(line[3:].split())
            if active and active[0] in ("lyrics", "voiceover") and len(active) != 3:
                raise ValueError(f"invalid block header: {line}")
    if active is not None:
        raise ValueError("unterminated code block")
    expected = {
        *(("lyrics", language, version) for language in LANGUAGES for version in VERSIONS),
        *(("voiceover", language, part) for language in LANGUAGES for part in ("intro", "outro")),
    }
    if set(blocks) != expected:
        raise ValueError(
            f"expected four lyric and four voiceover blocks; missing {expected - blocks.keys()}, "
            f"unexpected {blocks.keys() - expected}"
        )
    return blocks


def parse_sections(text: str) -> list[tuple[str, str]]:
    sections = []
    name = None
    lines = []
    for line in text.splitlines():
        if len(line) > LYRIC_LINE_LIMIT:
            raise ValueError(f"lyric line exceeds {LYRIC_LINE_LIMIT} characters")
        header = re.fullmatch(r"\[([^\[\]]+)\]", line)
        if header:
            if name is not None:
                sections.append((name, "\n".join(lines)))
            name, lines = header[1], [line]
            continue
        if name is None:
            raise ValueError("lyrics must start with a [Section name] header")
        lines.append(line)
        if len(lines) - 1 > SECTION_LINE_LIMIT:
            raise ValueError(f"section {name} exceeds {SECTION_LINE_LIMIT} lyric lines")
    if name is not None:
        sections.append((name, "\n".join(lines)))
    return sections


def storyboard_rows(text: str, version: str) -> list[list[str]]:
    heading = f"### {version.capitalize()} version"
    if text.splitlines().count(heading) != 1:
        raise ValueError(f"expected one storyboard heading: {heading}")
    section = text.split(heading + "\n", 1)[1].split("\n#", 1)[0]
    rows = [
        [cell.strip() for cell in line.strip().strip("|").split("|")]
        for line in section.splitlines() if line.startswith("|")
    ]
    if len(rows) < 3 or rows[0] != ["Section", "Start (s)", "End (s)", "On screen", "Sound effect"]:
        raise ValueError(f"invalid {version} storyboard table")
    return rows[2:]


def validate_storyboard(text: str, version: str) -> list[list[str]]:
    rows = storyboard_rows(text, version)
    chunks = MUSIC[version]["chunks"]
    if len(rows) != len(chunks):
        raise ValueError(f"{version} storyboard section count differs from music")
    start_ms = 0
    for row, chunk in zip(rows, chunks):
        end_ms = start_ms + chunk["duration_ms"]
        try:
            matches = (
                len(row) == 5 and row[0] == chunk["section"]
                and Decimal(row[1]) * 1000 == start_ms
                and Decimal(row[2]) * 1000 == end_ms
            )
        except InvalidOperation:
            matches = False
        if not matches:
            raise ValueError(f"{version} storyboard timing differs at {chunk['section']}")
        if row[4] != "none" and row[4] not in SOUND_EFFECTS:
            raise ValueError(f"unknown storyboard sound effect: {row[4]}")
        start_ms = end_ms
    return rows


def validate_document(text: str) -> dict:
    blocks = parse_document(text)
    storyboards = {}
    for version in VERSIONS:
        chunks = MUSIC[version]["chunks"]
        for language in LANGUAGES:
            names = [name for name, _ in parse_sections(blocks["lyrics", language, version])]
            if names != [chunk["section"] for chunk in chunks]:
                raise ValueError(f"{language} {version} lyric sections differ from music plan")
        storyboards[version] = validate_storyboard(text, version)
    for language in LANGUAGES:
        for part in ("intro", "outro"):
            spoken = blocks["voiceover", language, part]
            if (not spoken.strip() or len(spoken) >= VOICEOVER_CHARACTER_LIMIT
                    or re.search(r"<[^>]+>", spoken)):
                raise ValueError(
                    f"{language} {part} voiceover must be nonempty, "
                    f"under {VOICEOVER_CHARACTER_LIMIT} characters, without SSML"
                )
    blocks["storyboards"] = storyboards
    return blocks


def music_payload(blocks: dict, language: str, version: str, model: str) -> dict:
    sections = parse_sections(blocks["lyrics", language, version])
    chunks = []
    for index, ((_, text), chunk) in enumerate(zip(sections, MUSIC[version]["chunks"])):
        styles = list(chunk["positive_styles"])
        if index == 0:
            styles.append("German lyrics" if language == "de" else "English lyrics")
        chunks.append({
            "text": text,
            "duration_ms": chunk["duration_ms"],
            "positive_styles": styles,
            "negative_styles": list(chunk["negative_styles"]),
            "context_adherence": "high",
        })
    return {
        "model_id": model,
        "composition_plan": {"chunks": chunks},
        "seed": MUSIC[version]["seed"],
    }


def voice_payload(blocks: dict, language: str, part: str, model: str) -> dict:
    settings = VOICES[language]["voice_settings"]
    if model == "eleven_v4":
        settings = {key: settings[key] for key in ("stability", "similarity_boost")}
    return {
        "text": blocks["voiceover", language, part],
        "model_id": model,
        "language_code": language,
        "voice_settings": dict(settings),
        "seed": TTS_SEED,
    }


def sfx_payload(effect_id: str) -> dict:
    return {"model_id": SFX_MODEL, **SOUND_EFFECTS[effect_id]}


def selection(args: argparse.Namespace) -> tuple[tuple[str, ...], tuple[str, ...]]:
    languages = LANGUAGES if args.language == "both" else (args.language,)
    versions = VERSIONS if args.version == "both" else (args.version,)
    return languages, versions


def build_jobs(blocks: dict, args: argparse.Namespace) -> list[Job]:
    languages, versions = selection(args)
    jobs = []
    for language in languages:
        if "music" in args.parts:
            for version in versions:
                jobs.append(Job(
                    f"{APP_SLUG}-jingle-{version}-{language}.mp3",
                    f"/v1/music?output_format={MUSIC['output_format']}",
                    music_payload(blocks, language, version, args.music_model), MUSIC_TIMEOUT_S,
                ))
        if "voice" in args.parts:
            voice_id = VOICES[language]["voice_id"]
            for part in ("intro", "outro"):
                jobs.append(Job(
                    f"{APP_SLUG}-voiceover-{part}-{language}.mp3",
                    f"/v1/text-to-speech/{voice_id}?output_format={TTS_OUTPUT_FORMAT}",
                    voice_payload(blocks, language, part, args.tts_model),
                    REQUEST_TIMEOUT_S, voice_id,
                ))
    if "sfx" in args.parts:
        jobs.extend(Job(
            f"{APP_SLUG}-sfx-{effect_id}.mp3",
            f"/v1/sound-generation?output_format={SFX_OUTPUT_FORMAT}",
            sfx_payload(effect_id), REQUEST_TIMEOUT_S,
        ) for effect_id in SOUND_EFFECTS)
    return jobs


def http_request(
    method: str, endpoint: str, body: dict | None, api_key: str, timeout: int,
) -> tuple[bytes, dict]:
    request = urllib.request.Request(
        API_BASE + endpoint, method=method,
        data=None if body is None else json.dumps(body, ensure_ascii=False).encode("utf-8"),
        headers={"xi-api-key": api_key, "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return response.read(), dict(response.headers)
    except urllib.error.HTTPError as error:
        try:
            detail = error.read().decode("utf-8", errors="replace")
        except (OSError, http.client.HTTPException) as read_error:
            raise RuntimeError(f"HTTP {error.code} {method} {endpoint}: {read_error}") from None
        try:
            decoded = json.loads(detail)
            detail = json.dumps(decoded.get("detail", decoded), ensure_ascii=False)
        except (ValueError, AttributeError):
            pass
        raise RuntimeError(f"HTTP {error.code} {method} {endpoint}: {detail}") from None
    except (urllib.error.URLError, TimeoutError, http.client.HTTPException) as error:
        raise RuntimeError(f"{method} {endpoint} failed: {error}") from None


def write_file(path: Path, data: bytes) -> None:
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_bytes(data)
    temporary.chmod(0o644)
    temporary.replace(path)


def write_json(path: Path, value: dict) -> None:
    write_file(path, (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8"))


def file_record(job: Job, doc_hash: str, audio: bytes, headers: dict | None = None) -> dict:
    return {
        "endpoint": job.endpoint,
        "request_body": job.body,
        "model": job.body["model_id"],
        "seed": job.body.get("seed"),
        "response_headers": {
            key: value for key, value in (headers or {}).items() if key.lower() in RESPONSE_HEADERS
        },
        "doc_sha256": doc_hash,
        "sha256": hashlib.sha256(audio).hexdigest(),
        "size": len(audio),
    }


def load_manifest(path: Path) -> dict:
    if not path.exists():
        return {"app": APP_SLUG, "files": {}}
    manifest = json.loads(path.read_text(encoding="utf-8"))
    if (not isinstance(manifest, dict) or manifest.get("app") != APP_SLUG
            or not isinstance(manifest.get("files"), dict)):
        raise ValueError(f"invalid manifest: {path}")
    return manifest


def generate(jobs: list[Job], out_dir: Path, doc_hash: str, api_key: str,
             dry_run: bool, force: bool, transport) -> None:
    manifest_path = out_dir / "manifest.json"
    manifest = load_manifest(manifest_path)
    if dry_run:
        out_dir.mkdir(parents=True, exist_ok=True)
        for job in jobs:
            request_path = (out_dir / job.filename).with_suffix(".request.json")
            write_json(request_path, job.body)
            print(f"Request: {request_path}")
            print(f"Planned audio: {out_dir / job.filename}")
        write_json(manifest_path, manifest)
        print(f"Manifest: {manifest_path}")
        return
    pending = [job for job in jobs if force or not (out_dir / job.filename).exists()]
    voices = {voice["voice_id"]: voice for voice in VOICES.values()}
    for voice_id in dict.fromkeys(job.voice_id for job in pending if job.voice_id):
        try:
            transport("GET", f"/v1/voices/{voice_id}", None, api_key, REQUEST_TIMEOUT_S)
        except RuntimeError as error:
            raise RuntimeError(
                f"Voice {voices[voice_id]['name']} ({voice_id}) unavailable; "
                f"no audio requested: {error}"
            ) from None
    out_dir.mkdir(parents=True, exist_ok=True)
    for job in jobs:
        output = out_dir / job.filename
        if output.exists() and not force:
            print(f"Skip existing: {output}")
            if job.filename not in manifest["files"]:
                entry = file_record(job, doc_hash, output.read_bytes())
                entry["status"] = "existing"
                manifest["files"][job.filename] = entry
            continue
        audio, headers = transport("POST", job.endpoint, job.body, api_key, job.timeout)
        if not audio:
            raise RuntimeError(f"empty audio response for {job.filename}")
        write_file(output, audio)
        manifest["files"][job.filename] = file_record(job, doc_hash, audio, headers)
        write_json(manifest_path, manifest)
        print(f"Generated: {output}")
    write_json(manifest_path, manifest)
    print(f"Manifest: {manifest_path}")


def mix_plan(blocks: dict, language: str, version: str) -> dict:
    return {
        "filename": f"{APP_SLUG}-jingle-{version}-{language}-final.mp3",
        "intro": f"{APP_SLUG}-voiceover-intro-{language}.mp3",
        "music": f"{APP_SLUG}-jingle-{version}-{language}.mp3",
        "outro": f"{APP_SLUG}-voiceover-outro-{language}.mp3",
        "effects": [
            (f"{APP_SLUG}-sfx-{effect}.mp3", float(start))
            for _, start, _, _, effect in blocks["storyboards"][version]
            if effect != "none"
        ],
    }


def mix_inputs(plan: dict) -> list[str]:
    return [plan["intro"], plan["music"], plan["outro"], *(name for name, _ in plan["effects"])]


def ffmpeg_command(plan: dict, out_dir: Path, target: Path) -> list[str]:
    command = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y"]
    for name in mix_inputs(plan):
        command += ["-i", str(out_dir / name)]
    audio_format = f"aformat=sample_rates={MIX_SAMPLE_RATE}:channel_layouts=stereo"
    loudness = (f"loudnorm=I={MIX_LOUDNESS_LUFS}:TP={MIX_TRUE_PEAK_DB}"
                f":LRA={MIX_LOUDNESS_RANGE},{audio_format}")
    filters = [f"[{index}]{audio_format}[a{index}]" for index in range(3)]
    bed = ["[a1]"]
    for index, (_, start) in enumerate(plan["effects"], start=3):
        delay_ms = round(start * 1000)
        filters.append(
            f"[{index}]{audio_format},volume={MIX_SFX_GAIN},adelay={delay_ms}:all=1[a{index}]"
        )
        bed.append(f"[a{index}]")
    filters.append(
        f"{''.join(bed)}amix=inputs={len(bed)}:duration=first:normalize=0,{loudness}[bed]"
    )
    filters += [f"[a0]{loudness}[intro]", f"[a2]{loudness}[outro]"]
    gap = f"anullsrc=r={MIX_SAMPLE_RATE}:cl=stereo,atrim=duration={MIX_GAP_SECONDS}"
    filters += [f"{gap}[g0]", f"{gap}[g1]"]
    filters.append(
        f"[intro][g0][bed][g1][outro]concat=n=5:v=0:a=1,alimiter=limit={MIX_LIMIT}[out]"
    )
    return command + [
        "-filter_complex", ";".join(filters), "-map", "[out]", "-b:a", MIX_BITRATE,
        "-f", "mp3", str(target),
    ]


def run_ffmpeg(command: list[str]) -> None:
    if shutil.which(command[0]) is None:
        raise RuntimeError(FFMPEG_MISSING)
    try:
        result = subprocess.run(command, capture_output=True, text=True, timeout=FFMPEG_TIMEOUT_S)
    except subprocess.TimeoutExpired as error:
        detail = error.stderr or b""
        if isinstance(detail, bytes):
            detail = detail.decode("utf-8", errors="replace")
        raise RuntimeError(
            f"ffmpeg timed out after {FFMPEG_TIMEOUT_S} seconds: {detail.strip()[-500:]}"
        ) from None
    if result.returncode:
        raise RuntimeError(f"ffmpeg failed: {result.stderr.strip()[-500:]}")


def mix(blocks: dict, args: argparse.Namespace, out_dir: Path, mixer) -> None:
    languages, versions = selection(args)
    plans = [mix_plan(blocks, language, version) for language in languages for version in versions]
    if args.dry_run:
        for plan in plans:
            print(f"Plan {out_dir / plan['filename']} (mix of {', '.join(mix_inputs(plan))})")
        return
    missing = sorted({
        name for plan in plans for name in mix_inputs(plan) if not (out_dir / name).is_file()
    })
    if missing:
        raise ValueError(f"Mix needs {', '.join(missing)}; generate them with --parts music,voice,sfx")
    for plan in plans:
        path = out_dir / plan["filename"]
        temporary = path.with_suffix(path.suffix + ".tmp")
        try:
            mixer(ffmpeg_command(plan, out_dir, temporary))
            temporary.chmod(0o644)
            temporary.replace(path)
        finally:
            temporary.unlink(missing_ok=True)
        print(f"Mixed: {path}")


def parse_parts(value: str) -> tuple[str, ...]:
    parts = tuple(part.strip() for part in value.split(","))
    if not parts or any(part not in PARTS for part in parts):
        raise argparse.ArgumentTypeError("parts must be a comma list of music,voice,sfx,mix")
    return parts


def argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--api-key", help="ElevenLabs key; defaults to ELEVENLABS_API_KEY")
    parser.add_argument("--language", choices=(*LANGUAGES, "both"), default="both")
    parser.add_argument("--version", choices=(*VERSIONS, "both"), default="both")
    parser.add_argument("--parts", type=parse_parts, default=PARTS)
    parser.add_argument("--out-dir", type=Path, default=ROOT / "out/jingle")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--force", action="store_true")
    parser.add_argument("--tts-model", default=TTS_MODEL)
    parser.add_argument("--music-model", default=MUSIC["model_id"])
    return parser


def main(argv: list[str] | None = None, transport=None, mixer=None) -> int:
    args = argument_parser().parse_args(argv)
    api_key = (args.api_key or os.environ.get("ELEVENLABS_API_KEY", "")).strip()
    paid = any(part in PAID_PARTS for part in args.parts)
    if paid and not args.dry_run and not api_key:
        print("error: --api-key or ELEVENLABS_API_KEY is required unless --dry-run is set",
              file=sys.stderr)
        return 1
    try:
        source = DOC.read_bytes()
        blocks = validate_document(source.decode("utf-8"))
        out_dir = args.out_dir.resolve()
        mixer = mixer or run_ffmpeg
        if "mix" in args.parts and not args.dry_run and mixer is run_ffmpeg:
            if shutil.which("ffmpeg") is None:
                raise RuntimeError(FFMPEG_MISSING)
        if paid:
            generate(build_jobs(blocks, args), out_dir, hashlib.sha256(source).hexdigest(),
                     api_key, args.dry_run, args.force, transport or http_request)
        if "mix" in args.parts:
            mix(blocks, args, out_dir, mixer)
        return 0
    except (OSError, ValueError, RuntimeError) as error:
        message = str(error).replace(api_key, "[redacted]") if api_key else str(error)
        print(f"error: {message}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
