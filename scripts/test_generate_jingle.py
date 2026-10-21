from __future__ import annotations

import contextlib
import copy
import hashlib
import http.client
import importlib.util
import io
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
import urllib.error
from fractions import Fraction
from pathlib import Path
from unittest import mock

sys.dont_write_bytecode = True

SCRIPT = Path(__file__).with_name("generate-jingle.py").resolve()
SPEC = importlib.util.spec_from_file_location("generate_jingle", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
jingle = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = jingle
SPEC.loader.exec_module(jingle)

LINK_SPEC = importlib.util.spec_from_file_location(
    "jingle_markdown_links",
    jingle.ROOT / "templates/common/__strict__/scripts/check-markdown-links.py",
)
assert LINK_SPEC is not None and LINK_SPEC.loader is not None
links = importlib.util.module_from_spec(LINK_SPEC)
LINK_SPEC.loader.exec_module(links)


class JingleTests(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory(prefix="baukit-jingle-test-")
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.out_dir = self.directory / "audio"
        self.source = jingle.DOC.read_text(encoding="utf-8")
        self.blocks = jingle.validate_document(self.source)
        self.mixer = mock.Mock(side_effect=lambda command: Path(command[-1]).write_bytes(b"mixed audio"))
        patcher = mock.patch.object(
            jingle.urllib.request, "urlopen", side_effect=AssertionError("network forbidden")
        )
        self.network = patcher.start()
        self.addCleanup(patcher.stop)
        patcher = mock.patch.dict(os.environ, {"ELEVENLABS_API_KEY": ""})
        patcher.start()
        self.addCleanup(patcher.stop)

    def run_main(self, *arguments: str, transport=None, mixer=None) -> tuple[int, str, str]:
        stdout, stderr = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            status = jingle.main(
                ["--out-dir", str(self.out_dir), *arguments], transport=transport,
                mixer=self.mixer if mixer is None else mixer,
            )
        return status, stdout.getvalue(), stderr.getvalue()

    def manifest(self) -> dict:
        return json.loads((self.out_dir / "manifest.json").read_text())

    def jobs(self, *arguments: str) -> list:
        return jingle.build_jobs(self.blocks, jingle.argument_parser().parse_args(arguments))

    def fake_transport(self):
        return mock.Mock(return_value=(b"offline audio", {
            "Song-Id": "song-123",
            "Request-Id": "request-123",
            "Character-Cost": "37",
            "unrelated-header": "ignored",
        }))

    def replace_block(self, key: tuple, text: str) -> str:
        opening = "```" + " ".join(key) + "\n"
        return self.source.replace(opening + self.blocks[key], opening + text, 1)

    def test_music_constants_durations_and_bar_alignment(self) -> None:
        for version in jingle.VERSIONS:
            plan = jingle.MUSIC[version]
            chunks = plan["chunks"]
            self.assertTrue(1 <= len(chunks) <= jingle.MAX_CHUNKS)
            self.assertIsInstance(plan["seed"], int)
            low, high = jingle.VERSION_DURATION_RANGES[version]
            self.assertTrue(low <= sum(chunk["duration_ms"] for chunk in chunks) <= high)
            matches = [re.fullmatch(r"(\d+) BPM", tag) for tag in chunks[0]["positive_styles"]]
            bpm = int(next(match[1] for match in matches if match))
            bar_ms = Fraction(jingle.BEATS_PER_BAR * 60000, bpm)
            for chunk in chunks:
                with self.subTest(version=version, section=chunk["section"]):
                    low, high = jingle.CHUNK_DURATION_RANGE
                    self.assertTrue(low <= chunk["duration_ms"] <= high)
                    self.assertEqual(chunk["duration_ms"] % bar_ms, 0)
                    self.assertTrue(all(tag and tag.isascii() for tag in (
                        *chunk["positive_styles"], *chunk["negative_styles"],
                    )))

    def test_sung_lines_have_room_and_stay_below_word_density_limit(self) -> None:
        for language in jingle.LANGUAGES:
            for version in jingle.VERSIONS:
                sections = jingle.parse_sections(self.blocks["lyrics", language, version])
                for (_, text), chunk in zip(sections, jingle.MUSIC[version]["chunks"]):
                    sung = [re.sub(r"\{[^}]*\}", "", line).strip()
                            for line in text.splitlines()[1:]]
                    sung = [line for line in sung if line]
                    with self.subTest(language=language, version=version, section=chunk["section"]):
                        seconds = chunk["duration_ms"] / 1000
                        words = re.findall(r"\b\w+\b", " ".join(sung))
                        self.assertLessEqual(len(words) / seconds, 2.0)
                        bars = Fraction(chunk["duration_ms"] * jingle.MUSIC_BPM,
                                        jingle.BEATS_PER_BAR * 60000)
                        self.assertLessEqual(len(sung), bars / 2)
                        if seconds < 8:
                            self.assertLessEqual(len(sung), 1)

    def test_storyboard_timings_effect_ids_and_renamed_effects(self) -> None:
        for version in jingle.VERSIONS:
            jingle.validate_storyboard(self.source, version)
            row = jingle.storyboard_rows(self.source, version)[0]
            original = "| " + " | ".join(row) + " |"
            for field, value in ((0, "unknown section"), (1, "0.1"), (2, "invalid"),
                                 (4, "unknown-effect")):
                modified = row.copy()
                modified[field] = value
                source = self.source.replace(original, "| " + " | ".join(modified) + " |", 1)
                with self.assertRaisesRegex(ValueError, "storyboard"):
                    jingle.validate_document(source)
        renamed = {"renamed-" + name: effect for name, effect in jingle.SOUND_EFFECTS.items()}
        with mock.patch.object(jingle, "SOUND_EFFECTS", renamed):
            with self.assertRaisesRegex(ValueError, "sound effect"):
                jingle.validate_document(self.source)

    def test_invalid_document_is_rejected_before_requests_even_for_unselected_copy(self) -> None:
        lyric_key, voice_key = ("lyrics", "en", "long"), ("voiceover", "de", "outro")
        invalid = (
            self.replace_block(lyric_key, self.blocks[lyric_key].replace("[Intro]", "[Wrong]", 1)),
            self.replace_block(lyric_key, "[Intro]\n" + "x" * (jingle.LYRIC_LINE_LIMIT + 1)),
            self.replace_block(lyric_key, "[Intro]\n" + "\n".join(
                ["word"] * (jingle.SECTION_LINE_LIMIT + 1))),
            self.replace_block(lyric_key, "unheaded line\n" + self.blocks[lyric_key]),
            self.replace_block(voice_key, "x" * jingle.VOICEOVER_CHARACTER_LIMIT),
            self.replace_block(voice_key, "<speak>spoken text</speak>"),
            self.replace_block(voice_key, ""),
            self.source.replace("```voiceover de outro", "```voiceover de missing"),
            self.source + "\n```lyrics en short\n[Intro]\nDuplicate\n```\n",
            self.source + "\n```lyrics en\n",
            self.source + "\n```bash\nunterminated\n",
        )
        bad_doc = self.directory / "bad.md"
        for index, source in enumerate(invalid):
            with self.subTest(index=index):
                bad_doc.write_text(source, encoding="utf-8")
                with mock.patch.object(jingle, "DOC", bad_doc):
                    transport = self.fake_transport()
                    status, _, stderr = self.run_main(
                        "--api-key", "test-key", "--parts", "sfx", transport=transport,
                    )
                self.assertEqual(status, 1, stderr)
                transport.assert_not_called()
        self.network.assert_not_called()

    def test_instrumental_and_boundary_length_sections_parse(self) -> None:
        text = "[Break]\n{drum fill}\n[Verse]\n" + "\n".join(
            ["x" * jingle.LYRIC_LINE_LIMIT] * jingle.SECTION_LINE_LIMIT
        )
        self.assertEqual([name for name, _ in jingle.parse_sections(text)], ["Break", "Verse"])
        self.assertEqual(jingle.parse_sections("[Break]"), [("Break", "[Break]")])

    def test_music_payload_shape_text_and_language_styles(self) -> None:
        original = copy.deepcopy(jingle.MUSIC)
        for language in jingle.LANGUAGES:
            for version in jingle.VERSIONS:
                payload = jingle.music_payload(
                    self.blocks, language, version, jingle.MUSIC["model_id"],
                )
                self.assertEqual(set(payload), {"model_id", "composition_plan", "seed"})
                self.assertEqual(set(payload["composition_plan"]), {"chunks"})
                self.assertEqual(payload["seed"], jingle.MUSIC[version]["seed"])
                chunks = payload["composition_plan"]["chunks"]
                sections = jingle.parse_sections(self.blocks["lyrics", language, version])
                self.assertEqual([chunk["text"] for chunk in chunks],
                                 [text for _, text in sections])
                tag = "German lyrics" if language == "de" else "English lyrics"
                self.assertIn(tag, chunks[0]["positive_styles"])
                for chunk in chunks:
                    self.assertEqual(set(chunk), {
                        "text", "duration_ms", "positive_styles", "negative_styles",
                        "context_adherence",
                    })
                    self.assertIn(chunk["context_adherence"], ("low", "medium", "high"))
                chunks[0]["positive_styles"].append("test mutation")
        self.assertEqual(jingle.MUSIC, original)

    def test_tts_presets_and_model_specific_settings(self) -> None:
        for language, voice in jingle.VOICES.items():
            self.assertTrue(voice["name"])
            settings = voice["voice_settings"]
            for key in ("stability", "similarity_boost", "style"):
                self.assertTrue(0 <= settings[key] <= 1)
            self.assertIsInstance(settings["use_speaker_boost"], bool)
            self.assertTrue(0.7 <= settings["speed"] <= 1.2)
            for model in ("eleven_v4", "eleven_v3"):
                payload = jingle.voice_payload(self.blocks, language, "intro", model)
                self.assertEqual(set(payload), {
                    "text", "model_id", "language_code", "voice_settings", "seed",
                })
                expected = ({key: settings[key] for key in ("stability", "similarity_boost")}
                            if model == "eleven_v4" else settings)
                self.assertEqual(payload["voice_settings"], expected)
                self.assertEqual(payload["language_code"], language)
                self.assertEqual(payload["model_id"], model)
                self.assertEqual(payload["text"], self.blocks["voiceover", language, "intro"])
                self.assertIsInstance(payload["seed"], int)

    def test_sfx_payloads_and_independent_output_formats(self) -> None:
        for effect_id in jingle.SOUND_EFFECTS:
            payload = jingle.sfx_payload(effect_id)
            self.assertEqual(set(payload), {
                "text", "model_id", "duration_seconds", "prompt_influence",
            })
            self.assertEqual(payload["model_id"], "eleven_text_to_sound_v2")
            self.assertTrue(payload["text"].strip())
            low, high = jingle.SFX_DURATION_RANGE
            self.assertTrue(low <= payload["duration_seconds"] <= high)
            low, high = jingle.SFX_INFLUENCE_RANGE
            self.assertTrue(low <= payload["prompt_influence"] <= high)
        with mock.patch.object(jingle, "TTS_OUTPUT_FORMAT", "speech-format"), \
                mock.patch.object(jingle, "SFX_OUTPUT_FORMAT", "effect-format"):
            for job in self.jobs("--parts", "voice,sfx"):
                expected = "speech-format" if job.voice_id else "effect-format"
                self.assertTrue(job.endpoint.endswith("output_format=" + expected))

    def test_dry_run_writes_requests_without_audio_records_or_network(self) -> None:
        for force in ((), ("--force",)):
            status, stdout, stderr = self.run_main("--dry-run", *force)
            self.assertEqual(status, 0, stderr)
            for job in self.jobs():
                path = (self.out_dir / job.filename).with_suffix(".request.json")
                self.assertEqual(json.loads(path.read_text()), job.body)
                self.assertIn(str(path), stdout)
                self.assertIn(job.filename, stdout)
            self.assertEqual(self.manifest()["files"], {})
            self.assertEqual(list(self.out_dir.glob("*.mp3")), [])
        self.network.assert_not_called()

    def test_language_version_parts_and_model_overrides(self) -> None:
        status, _, stderr = self.run_main(
            "--dry-run", "--language", "de", "--version", "short", "--parts", " music,voice ",
            "--tts-model", "eleven_v3", "--music-model", "test-music-model",
        )
        self.assertEqual(status, 0, stderr)
        self.assertEqual({path.name for path in self.out_dir.glob("*.request.json")}, {
            "baukit-jingle-short-de.request.json",
            "baukit-voiceover-intro-de.request.json",
            "baukit-voiceover-outro-de.request.json",
        })
        for path in self.out_dir.glob("*.request.json"):
            model = "test-music-model" if "jingle" in path.name else "eleven_v3"
            self.assertEqual(json.loads(path.read_text())["model_id"], model)

    def test_missing_key_fails_before_network_or_output(self) -> None:
        status, _, stderr = self.run_main()
        self.assertEqual(status, 1)
        self.assertIn("--api-key or ELEVENLABS_API_KEY is required", stderr)
        self.network.assert_not_called()
        self.assertFalse(self.out_dir.exists())

    def test_key_whitespace_is_stripped_before_http_headers_and_empty_keys_fail(self) -> None:
        secret = "trimmed-test-key"
        response = mock.MagicMock()
        response.__enter__.return_value = response
        response.read.return_value = b"audio"
        response.headers = {}
        for source in ("cli", "env"):
            for value in (f"\t {secret}\r\n", " \t\r\n"):
                with self.subTest(source=source, empty=not value.strip()):
                    arguments = ("--api-key", value) if source == "cli" else ()
                    self.network.reset_mock()
                    self.network.side_effect = None
                    self.network.return_value = response
                    with mock.patch.dict(os.environ, {
                        "ELEVENLABS_API_KEY": value if source == "env" else "",
                    }):
                        status, stdout, stderr = self.run_main(
                            *arguments, "--parts", "sfx", "--force",
                        )
                    self.assertNotIn(secret, stdout + stderr)
                    if value.strip():
                        self.assertEqual(status, 0, stderr)
                        self.network.assert_called()
                        for call in self.network.call_args_list:
                            self.assertEqual(call.args[0].get_header("Xi-api-key"), secret)
                    else:
                        self.assertEqual(status, 1)
                        self.assertIn("required", stderr)
                        self.network.assert_not_called()

    def test_preflight_checks_voices_before_paid_calls_and_names_failed_voice(self) -> None:
        for distinct, fail in ((False, False), (True, False), (True, True)):
            with self.subTest(distinct=distinct, fail=fail):
                self.out_dir = self.directory / f"voices-{distinct}-{fail}"
                voices = copy.deepcopy(jingle.VOICES)
                if distinct:
                    voices["de"] = {
                        **voices["de"], "voice_id": "another-id", "name": "Other foreman",
                    }
                transport = self.fake_transport()
                if fail:
                    transport.side_effect = [(b"{}", {}), RuntimeError("HTTP 404 voice_not_found")]
                with mock.patch.object(jingle, "VOICES", voices):
                    status, _, stderr = self.run_main("--api-key", "test-key", transport=transport)
                calls = [call.args for call in transport.call_args_list]
                ids = list(dict.fromkeys(voice["voice_id"] for voice in voices.values()))
                self.assertEqual([call[1] for call in calls[:len(ids)]],
                                 [f"/v1/voices/{voice_id}" for voice_id in ids])
                self.assertTrue(all(call[0] == "GET" and call[2] is None
                                    for call in calls[:len(ids)]))
                if fail:
                    self.assertEqual(status, 1)
                    self.assertIn(voices["de"]["name"], stderr)
                    self.assertIn("voice_not_found", stderr)
                    self.assertEqual(len(calls), len(ids))
                    self.assertFalse(self.out_dir.exists())
                else:
                    self.assertEqual(status, 0, stderr)
                    self.assertEqual(len(calls), len(ids) + len(self.jobs()))
                    self.assertTrue(all(call[0] == "POST" for call in calls[len(ids):]))

    def test_music_and_effects_do_not_need_voice_preflight(self) -> None:
        transport = self.fake_transport()
        status, _, stderr = self.run_main(
            "--api-key", "test-key", "--parts", "music,sfx", transport=transport,
        )
        self.assertEqual(status, 0, stderr)
        self.assertTrue(all(call.args[0] == "POST" for call in transport.call_args_list))

    def test_manifest_records_inputs_hashes_sizes_headers_and_timeouts(self) -> None:
        transport = self.fake_transport()
        status, _, stderr = self.run_main("--api-key", "test-key", transport=transport)
        self.assertEqual(status, 0, stderr)
        entries = self.manifest()["files"]
        for job in self.jobs():
            audio = (self.out_dir / job.filename).read_bytes()
            self.assertEqual(entries[job.filename], {
                "endpoint": job.endpoint,
                "request_body": job.body,
                "model": job.body["model_id"],
                "seed": job.body.get("seed"),
                "response_headers": {
                    "Song-Id": "song-123",
                    "Request-Id": "request-123",
                    "Character-Cost": "37",
                },
                "doc_sha256": hashlib.sha256(jingle.DOC.read_bytes()).hexdigest(),
                "sha256": hashlib.sha256(audio).hexdigest(),
                "size": len(audio),
            })
            timeout = (jingle.MUSIC_TIMEOUT_S if job.endpoint.startswith("/v1/music?")
                       else jingle.REQUEST_TIMEOUT_S)
            transport.assert_any_call("POST", job.endpoint, job.body, "test-key", timeout)

    def test_skip_force_and_dry_run_preserve_existing_generation_records(self) -> None:
        status, _, stderr = self.run_main("--api-key", "test-key", transport=self.fake_transport())
        self.assertEqual(status, 0, stderr)
        original = self.manifest()
        modified = self.directory / "modified.md"
        modified.write_text(self.source + "\nAn editing note.\n", encoding="utf-8")
        transport = self.fake_transport()
        with mock.patch.object(jingle, "DOC", modified):
            status, _, stderr = self.run_main("--api-key", "test-key", transport=transport)
            self.assertEqual(status, 0, stderr)
            transport.assert_not_called()
            for force in ((), ("--force",)):
                status, _, stderr = self.run_main(
                    "--dry-run", "--music-model", "new-model", *force, transport=transport,
                )
                self.assertEqual(status, 0, stderr)
                self.assertEqual(self.manifest(), original)
                transport.assert_not_called()
        transport.return_value = (b"replacement audio", {})
        status, _, stderr = self.run_main(
            "--api-key", "test-key", "--force", "--parts", "voice", "--language", "de",
            "--tts-model", "eleven_v3", transport=transport,
        )
        self.assertEqual(status, 0, stderr)
        selected = {job.filename for job in self.jobs("--parts", "voice", "--language", "de")}
        updated = self.manifest()["files"]
        self.assertEqual(set(updated), set(original["files"]))
        for filename, entry in updated.items():
            if filename in selected:
                self.assertEqual(entry["model"], "eleven_v3")
                self.assertEqual((self.out_dir / filename).read_bytes(), b"replacement audio")
            else:
                self.assertEqual(entry, original["files"][filename])

    def test_existing_audio_without_entry_records_full_job_data(self) -> None:
        self.out_dir.mkdir()
        job = self.jobs("--parts", "sfx")[0]
        (self.out_dir / job.filename).write_bytes(b"existing audio")
        transport = self.fake_transport()
        status, _, stderr = self.run_main(
            "--api-key", "test-key", "--parts", "sfx", transport=transport,
        )
        self.assertEqual(status, 0, stderr)
        self.assertFalse(any(call.args[1] == job.endpoint and call.args[2] == job.body
                             for call in transport.call_args_list))
        expected = jingle.file_record(job, hashlib.sha256(jingle.DOC.read_bytes()).hexdigest(),
                                      b"existing audio")
        self.assertEqual(self.manifest()["files"][job.filename], {**expected, "status": "existing"})

    def test_failed_paid_request_keeps_completed_files_and_rerun_skips_them(self) -> None:
        transport = mock.Mock(side_effect=[(b"first", {}), RuntimeError("HTTP 429 quota_exceeded")])
        status, _, stderr = self.run_main(
            "--api-key", "test-key", "--parts", "sfx", transport=transport,
        )
        self.assertEqual(status, 1)
        self.assertIn("quota_exceeded", stderr)
        first = self.jobs("--parts", "sfx")[0]
        self.assertEqual(set(self.manifest()["files"]), {first.filename})
        transport = self.fake_transport()
        status, _, stderr = self.run_main(
            "--api-key", "test-key", "--parts", "sfx", transport=transport,
        )
        self.assertEqual(status, 0, stderr)
        self.assertEqual(transport.call_count, len(jingle.SOUND_EFFECTS) - 1)
        self.assertEqual((self.out_dir / first.filename).read_bytes(), b"first")

    def test_http_wrapper_sends_json_auth_format_and_timeout(self) -> None:
        response = mock.MagicMock()
        response.__enter__.return_value = response
        response.read.return_value = b"audio"
        response.headers = {"song-id": "a-song"}
        self.network.side_effect = None
        self.network.return_value = response
        body = {"text": "Fünfmal", "model_id": "test"}
        audio, headers = jingle.http_request(
            "POST", "/v1/music?output_format=test", body, "secret", jingle.MUSIC_TIMEOUT_S,
        )
        request = self.network.call_args.args[0]
        self.assertEqual(request.full_url, "https://api.elevenlabs.io/v1/music?output_format=test")
        self.assertEqual(request.get_method(), "POST")
        self.assertEqual(json.loads(request.data), body)
        self.assertEqual(request.get_header("Xi-api-key"), "secret")
        self.assertEqual(request.get_header("Content-type"), "application/json")
        self.assertEqual(self.network.call_args.kwargs, {"timeout": jingle.MUSIC_TIMEOUT_S})
        self.assertEqual((audio, headers), (b"audio", {"song-id": "a-song"}))

    def test_http_errors_surface_status_and_detail(self) -> None:
        for body in (json.dumps({"detail": "bad_composition_plan"}), "bad_composition_plan"):
            self.network.side_effect = urllib.error.HTTPError(
                jingle.API_BASE, 400, "Bad Request", {}, io.BytesIO(body.encode()),
            )
            status, _, stderr = self.run_main("--api-key", "test-key", "--parts", "music")
            self.assertEqual(status, 1)
            self.assertIn("HTTP 400", stderr)
            self.assertIn("bad_composition_plan", stderr)
            self.assertNotIn("Traceback", stderr)

    def test_transport_read_errors_and_empty_audio_fail_without_completed_output(self) -> None:
        for error in (http.client.IncompleteRead(b"partial", 100),
                      http.client.HTTPException("invalid response"), TimeoutError("timed out")):
            with self.subTest(error=type(error).__name__):
                response = mock.MagicMock()
                response.__enter__.return_value = response
                response.read.side_effect = error
                self.network.side_effect = None
                self.network.return_value = response
                status, _, stderr = self.run_main("--api-key", "test-key", "--parts", "sfx")
                self.assertEqual(status, 1)
                self.assertIn("failed", stderr)
                self.assertNotIn("Traceback", stderr)
                self.assertEqual(list(self.out_dir.glob("*.mp3")), [])
        transport = mock.Mock(return_value=(b"", {}))
        status, _, stderr = self.run_main(
            "--api-key", "test-key", "--parts", "sfx", transport=transport,
        )
        self.assertEqual(status, 1)
        self.assertIn("empty audio response", stderr)

    def test_keys_from_environment_and_cli_stay_out_of_artifacts_and_error_output(self) -> None:
        secret = "sensitive-test-key-123"
        for arguments in ((), ("--api-key", secret)):
            with mock.patch.dict(os.environ, {"ELEVENLABS_API_KEY": secret}):
                transport = self.fake_transport()
                status, stdout, stderr = self.run_main(*arguments, "--force", transport=transport)
                self.assertEqual(status, 0, stderr)
                self.assertTrue(all(call.args[3] == secret for call in transport.call_args_list))
                status, dry_stdout, dry_stderr = self.run_main(*arguments, "--dry-run", "--force")
                self.assertEqual(status, 0, dry_stderr)
                self.assertNotIn(secret, stdout + stderr + dry_stdout + dry_stderr)
                for path in self.out_dir.iterdir():
                    self.assertNotIn(secret.encode(), path.read_bytes())
                transport.side_effect = RuntimeError("HTTP 400 echoed " + secret)
                status, stdout, stderr = self.run_main(*arguments, "--force", transport=transport)
                self.assertEqual(status, 1)
                self.assertIn("[redacted]", stderr)
                self.assertNotIn(secret, stdout + stderr)

    def test_written_outputs_have_mode_0644_even_with_restrictive_umask(self) -> None:
        previous = os.umask(0o077)
        try:
            status, _, stderr = self.run_main(
                "--api-key", "test-key", transport=self.fake_transport(),
            )
            self.assertEqual(status, 0, stderr)
            status, _, stderr = self.run_main("--dry-run")
            self.assertEqual(status, 0, stderr)
        finally:
            os.umask(previous)
        for path in self.out_dir.iterdir():
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o644, path.name)

    def test_default_path_uses_script_location_but_explicit_paths_use_cwd(self) -> None:
        copied = self.directory / "repo/scripts/generate-jingle.py"
        copied.parent.mkdir(parents=True)
        copied.write_bytes(SCRIPT.read_bytes())
        document = copied.parents[1] / "docs/JINGLE.md"
        document.parent.mkdir()
        document.write_bytes(jingle.DOC.read_bytes())
        wrapper = (
            "import runpy, sys, urllib.request; "
            "urllib.request.urlopen = lambda *a, **k: sys.exit('network forbidden'); "
            "sys.argv = sys.argv[1:]; runpy.run_path(sys.argv[0], run_name='__main__')"
        )
        for arguments, expected in (
            ((), copied.parents[1] / "out/jingle"),
            (("--out-dir", "rendered"), self.directory / "rendered"),
            (("--out-dir", "out/jingle"), self.directory / "out/jingle"),
        ):
            result = subprocess.run(
                [sys.executable, "-c", wrapper, str(copied), "--dry-run", *arguments],
                cwd=self.directory, capture_output=True, text=True,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            self.assertTrue((expected / "manifest.json").exists())
            self.assertIn(str(expected), result.stdout)

    def test_mix_command_uses_storyboard_effects_and_delays_for_every_selection(self) -> None:
        for language in jingle.LANGUAGES:
            for version in jingle.VERSIONS:
                with self.subTest(language=language, version=version):
                    plan = jingle.mix_plan(self.blocks, language, version)
                    self.assertEqual(plan["filename"], f"baukit-jingle-{version}-{language}-final.mp3")
                    cues = [(row[4], float(row[1])) for row in jingle.storyboard_rows(self.source, version)
                            if row[4] != "none"]
                    names = [f"baukit-voiceover-intro-{language}.mp3",
                             f"baukit-jingle-{version}-{language}.mp3",
                             f"baukit-voiceover-outro-{language}.mp3",
                             *(f"baukit-sfx-{effect}.mp3" for effect, _ in cues)]
                    self.assertEqual(jingle.mix_inputs(plan), names)
                    expected_cues = {"short": [("anvil", 0.0), ("blueprint", 6.4), ("relay", 19.2), ("anvil", 38.4)],
                                     "long": [("anvil", 0.0), ("blueprint", 6.4), ("relay", 19.2), ("anvil", 64.0)]}
                    self.assertEqual(cues, expected_cues[version])
                    target = self.out_dir / (plan["filename"] + ".tmp")
                    command = jingle.ffmpeg_command(plan, self.out_dir, target)
                    self.assertEqual(command[:5], ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y"])
                    inputs = [command[index + 1] for index, item in enumerate(command) if item == "-i"]
                    self.assertEqual(inputs, [str(self.out_dir / name) for name in names])
                    graph = command[command.index("-filter_complex") + 1]
                    audio_format = "aformat=sample_rates=44100:channel_layouts=stereo"
                    loudness = f"loudnorm=I=-14:TP=-1.5:LRA=11,{audio_format}"
                    for index in range(3):
                        self.assertIn(f"[{index}]{audio_format}[a{index}]", graph)
                    for index, (_, start) in enumerate(cues, start=3):
                        self.assertIn(f"[{index}]{audio_format},volume=0.7,"
                                      f"adelay={round(start * 1000)}:all=1[a{index}]", graph)
                    self.assertEqual([int(value) for value in re.findall(r"adelay=(\d+):all=1", graph)],
                                     [round(start * 1000) for _, start in cues])
                    bed = "[a1]" + "".join(f"[a{index}]" for index in range(3, len(names)))
                    self.assertIn(f"{bed}amix=inputs={len(cues) + 1}:duration=first:normalize=0,"
                                  f"{loudness}[bed]", graph)
                    self.assertIn(f"[a0]{loudness}[intro]", graph)
                    self.assertIn(f"[a2]{loudness}[outro]", graph)
                    self.assertEqual(graph.count("loudnorm="), 3)
                    for index in range(2):
                        self.assertIn(f"anullsrc=r=44100:cl=stereo,atrim=duration=0.3[g{index}]", graph)
                    self.assertTrue(graph.endswith(
                        "[intro][g0][bed][g1][outro]concat=n=5:v=0:a=1,alimiter=limit=0.89[out]"
                    ))
                    self.assertEqual(command[-7:], ["-map", "[out]", "-b:a", "192k", "-f", "mp3", str(target)])

    def test_mix_omits_none_effects_and_rejects_empty_cells(self) -> None:
        source = self.source
        for effect in jingle.SOUND_EFFECTS:
            source = source.replace(f"| {effect} |", "| none |")
        blocks = jingle.validate_document(source)
        for version in jingle.VERSIONS:
            plan = jingle.mix_plan(blocks, "en", version)
            self.assertEqual(plan["effects"], [])
            command = jingle.ffmpeg_command(plan, self.out_dir, self.out_dir / "final.mp3")
            graph = command[command.index("-filter_complex") + 1]
            self.assertNotIn("adelay", graph)
            self.assertIn("[a1]amix=inputs=1:duration=first:normalize=0", graph)
        with self.assertRaisesRegex(ValueError, "unknown storyboard sound effect"):
            jingle.validate_document(self.source.replace("| relay |", "|  |", 1))

    def test_mix_alone_needs_no_key_or_network_and_lists_all_missing_inputs(self) -> None:
        transport = self.fake_transport()
        status, stdout, stderr = self.run_main("--parts", "mix", transport=transport)
        self.assertEqual(status, 1)
        self.assertEqual(stdout, "")
        names = sorted({job.filename for job in self.jobs()})
        self.assertIn(f"Mix needs {', '.join(names)}; generate them with --parts music,voice,sfx", stderr)
        self.assertFalse(self.out_dir.exists())
        self.mixer.assert_not_called()
        transport.assert_not_called()
        self.network.assert_not_called()

    def test_mix_checks_all_selected_inputs_before_writing_any_final(self) -> None:
        self.out_dir.mkdir()
        for job in self.jobs():
            if job.filename != "baukit-jingle-long-de.mp3":
                (self.out_dir / job.filename).write_bytes(b"earlier audio")
        status, _, stderr = self.run_main("--parts", "mix")
        self.assertEqual(status, 1)
        self.assertIn("Mix needs baukit-jingle-long-de.mp3;", stderr)
        self.mixer.assert_not_called()
        self.assertEqual(list(self.out_dir.glob("*-final.mp3")), [])

    def test_mix_alone_rebuilds_without_creating_or_changing_a_manifest(self) -> None:
        self.out_dir.mkdir()
        for job in self.jobs("--language", "de", "--version", "long"):
            (self.out_dir / job.filename).write_bytes(b"earlier audio")
        manifest = self.out_dir / "manifest.json"
        final = self.out_dir / "baukit-jingle-long-de-final.mp3"
        for existing_manifest in (None, b"an unrelated manifest that must not be read or written\n"):
            if existing_manifest is not None:
                manifest.write_bytes(existing_manifest)
            final.write_bytes(b"old final")
            transport = self.fake_transport()
            status, _, stderr = self.run_main(
                "--parts", "mix", "--language", "de", "--version", "long", transport=transport,
            )
            self.assertEqual(status, 0, stderr)
            self.assertEqual(final.read_bytes(), b"mixed audio")
            self.assertEqual(manifest.read_bytes() if manifest.exists() else None, existing_manifest)
            transport.assert_not_called()
        self.assertEqual(self.mixer.call_count, 2)
        self.assertEqual(list(self.out_dir.glob("*-final.mp3")), [final])
        self.assertEqual(list(self.out_dir.glob("*.tmp")), [])
        self.network.assert_not_called()

    def test_mix_dry_run_prints_exact_plans_without_writing(self) -> None:
        for language, version in (("both", "both"), ("en", "long"), ("de", "short")):
            with self.subTest(language=language, version=version):
                with mock.patch.object(jingle.shutil, "which", return_value=None):
                    status, stdout, stderr = self.run_main(
                        "--dry-run", "--parts", "mix", "--language", language, "--version", version,
                        mixer=jingle.run_ffmpeg,
                    )
                self.assertEqual(status, 0, stderr)
                plans = [jingle.mix_plan(self.blocks, lang, cut)
                         for lang in (jingle.LANGUAGES if language == "both" else (language,))
                         for cut in (jingle.VERSIONS if version == "both" else (version,))]
                self.assertEqual(stdout.splitlines(), [
                    f"Plan {self.out_dir / plan['filename']} (mix of {', '.join(jingle.mix_inputs(plan))})"
                    for plan in plans
                ])
                self.assertFalse(self.out_dir.exists())
        self.mixer.assert_not_called()
        self.network.assert_not_called()

    def test_full_run_generates_before_mixing_every_selection_and_dry_run_only_plans(self) -> None:
        status, stdout, stderr = self.run_main("--dry-run")
        self.assertEqual(status, 0, stderr)
        self.assertEqual(len([line for line in stdout.splitlines() if line.startswith("Plan ")]), 4)
        self.mixer.assert_not_called()
        self.assertEqual(list(self.out_dir.glob("*-final.mp3")), [])
        transport = self.fake_transport()

        def mixer(command):
            self.assertEqual(transport.call_count, len(self.jobs()) + 1)
            self.assertEqual(set(self.manifest()["files"]), {job.filename for job in self.jobs()})
            self.assertTrue(all((self.out_dir / job.filename).is_file() for job in self.jobs()))
            self.mixer(command)

        status, _, stderr = self.run_main("--api-key", "test-key", transport=transport, mixer=mixer)
        self.assertEqual(status, 0, stderr)
        self.assertEqual(self.mixer.call_count, 4)
        self.assertEqual(sorted(path.name for path in self.out_dir.glob("*-final.mp3")), sorted(
            f"baukit-jingle-{version}-{language}-final.mp3"
            for language in jingle.LANGUAGES for version in jingle.VERSIONS
        ))
        self.assertEqual(list(self.out_dir.glob("*.tmp")), [])

    def test_missing_ffmpeg_stops_before_any_paid_call(self) -> None:
        for mixer in (None, jingle.run_ffmpeg):
            transport = self.fake_transport()
            stderr = io.StringIO()
            with mock.patch.object(jingle.shutil, "which", return_value=None), \
                    contextlib.redirect_stderr(stderr):
                status = jingle.main(
                    ["--out-dir", str(self.out_dir), "--api-key", "test-key"],
                    transport=transport, mixer=mixer,
                )
            self.assertEqual(status, 1)
            self.assertIn(jingle.FFMPEG_MISSING, stderr.getvalue())
            self.assertFalse(self.out_dir.exists())
            transport.assert_not_called()
        self.network.assert_not_called()

    def test_ffmpeg_failure_and_timeout_report_stderr_and_remove_temporary_output(self) -> None:
        self.out_dir.mkdir()
        for job in self.jobs("--language", "en", "--version", "short"):
            (self.out_dir / job.filename).write_bytes(b"earlier audio")
        final = self.out_dir / "baukit-jingle-short-en-final.mp3"
        final.write_bytes(b"old final")
        manifest = self.out_dir / "manifest.json"
        manifest.write_bytes(b"untouched manifest")
        detail = "discarded-prefix" + "x" * 600 + "useful error tail"
        outcomes = (
            subprocess.CompletedProcess([], 1, "", detail),
            subprocess.TimeoutExpired("ffmpeg", jingle.FFMPEG_TIMEOUT_S, stderr=detail.encode()),
        )
        for outcome in outcomes:
            def runner(command, **kwargs):
                self.assertEqual(kwargs, {
                    "capture_output": True, "text": True, "timeout": jingle.FFMPEG_TIMEOUT_S,
                })
                self.assertEqual(Path(command[-1]), final.with_suffix(".mp3.tmp"))
                Path(command[-1]).write_bytes(b"partial mix")
                if isinstance(outcome, Exception):
                    raise outcome
                return outcome

            with mock.patch.object(jingle.shutil, "which", return_value="/test/ffmpeg"), \
                    mock.patch.object(jingle.subprocess, "run", side_effect=runner):
                status, _, stderr = self.run_main(
                    "--parts", "mix", "--language", "en", "--version", "short", mixer=jingle.run_ffmpeg,
                )
            self.assertEqual(status, 1)
            self.assertIn("useful error tail", stderr)
            self.assertNotIn("discarded-prefix", stderr)
            self.assertIn("timed out" if isinstance(outcome, Exception) else "ffmpeg failed", stderr)
            self.assertNotIn("Traceback", stderr)
            self.assertEqual(final.read_bytes(), b"old final")
            self.assertEqual(manifest.read_bytes(), b"untouched manifest")
            self.assertEqual(list(self.out_dir.glob("*.tmp")), [])

    @unittest.skipUnless(shutil.which("ffmpeg") and shutil.which("ffprobe"), "ffmpeg/ffprobe absent")
    def test_real_ffmpeg_mix_duration_with_generated_sine_inputs(self) -> None:
        self.out_dir.mkdir()
        for version in jingle.VERSIONS:
            with self.subTest(version=version):
                plan = jingle.mix_plan(self.blocks, "en", version)
                music_seconds = float(jingle.storyboard_rows(self.source, version)[-1][2])
                lengths = {plan["intro"]: 1.0, plan["music"]: music_seconds, plan["outro"]: 2.0}
                lengths.update({name: 0.5 for name, _ in plan["effects"]})
                for name, seconds in lengths.items():
                    subprocess.run([
                        "ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi",
                        "-i", f"sine=duration={seconds}:sample_rate=48000", str(self.out_dir / name),
                    ], check=True, capture_output=True, timeout=60)
                status, _, stderr = self.run_main(
                    "--parts", "mix", "--language", "en", "--version", version, mixer=jingle.run_ffmpeg,
                )
                self.assertEqual(status, 0, stderr)
                probe = subprocess.run([
                    "ffprobe", "-v", "error", "-show_entries",
                    "format=duration:stream=sample_rate,channels,bit_rate", "-of", "json",
                    str(self.out_dir / plan["filename"]),
                ], capture_output=True, text=True, check=True, timeout=60)
                audio = json.loads(probe.stdout)
                expected = 1.0 + music_seconds + 2 * jingle.MIX_GAP_SECONDS + 2.0
                self.assertAlmostEqual(float(audio["format"]["duration"]), expected, delta=0.2)
                self.assertEqual(audio["streams"][0], {
                    "sample_rate": "44100", "channels": 2, "bit_rate": "192000",
                })
                self.assertEqual(stat.S_IMODE((self.out_dir / plan["filename"]).stat().st_mode), 0o644)
                self.assertEqual(list(self.out_dir.glob("*.tmp")), [])
                self.assertFalse((self.out_dir / "manifest.json").exists())
        self.mixer.assert_not_called()
        self.network.assert_not_called()

    def test_invalid_cli_values_and_manifest_fail_before_network(self) -> None:
        for value in ("", "music,unknown", "music,,voice"):
            with contextlib.redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as caught:
                jingle.main(["--dry-run", "--parts", value])
            self.assertEqual(caught.exception.code, 2)
        self.out_dir.mkdir()
        (self.out_dir / "manifest.json").write_text('{"app":"other","files":{}}')
        transport = self.fake_transport()
        status, _, stderr = self.run_main("--api-key", "test-key", transport=transport)
        self.assertEqual(status, 1)
        self.assertIn("invalid manifest", stderr)
        transport.assert_not_called()
        self.network.assert_not_called()

    def test_links_resolve_and_generator_has_shebang_and_executable_bit(self) -> None:
        with mock.patch.object(links, "markdown_files", return_value=[jingle.DOC]):
            self.assertEqual(links.missing_links(jingle.ROOT, ["docs/JINGLE.md"]), [])
        self.assertTrue(os.access(SCRIPT, os.X_OK))
        self.assertEqual(SCRIPT.read_text().splitlines()[0], "#!/usr/bin/env python3")
        self.assertNotIn("\u2014", self.source)


if __name__ == "__main__":
    unittest.main()
