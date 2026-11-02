"""The information-phase search accepts plain public links only and tells a pair from one file."""

from __future__ import annotations

import unittest

import search_formats as search


VIDEO_ONLY = {"format_id": "v", "protocol": "https", "ext": "mp4", "vcodec": "avc1", "acodec": "none"}
AUDIO_ONLY = {"format_id": "a", "protocol": "https", "ext": "m4a", "vcodec": "none", "acodec": "mp4a"}
COMBINED = {"format_id": "c", "protocol": "https", "ext": "mp4", "vcodec": "avc1", "acodec": "mp4a"}


class CandidateTests(unittest.TestCase):
    def test_plain_page_links_pass(self) -> None:
        for link in (
            "https://video.example.org/videos/watch/abc",
            "https://www.example.com/watch?v=abc123",
        ):
            self.assertEqual(search.check_candidate(link), link)

    def test_links_with_credentials_tokens_or_other_shapes_are_refused(self) -> None:
        for link in (
            "http://video.example.org/a",
            "https://user:pass@video.example.org/a",
            "https://video.example.org:8443/a",
            "https://video.example.org/a?token=abc",
            "https://video.example.org/a?v=1&sig=abc",
            "https://video.example.org/a#t=3",
            "https://127.0.0.1/a",
            "https://localhost/a",
        ):
            with self.assertRaises(search.CandidateRefused, msg=link):
                search.check_candidate(link)


class ClassifyTests(unittest.TestCase):
    def test_separate_audio_and_video_are_a_pair(self) -> None:
        verdict = search.classify({"requested_formats": [VIDEO_ONLY, AUDIO_ONLY]})
        self.assertEqual(verdict["verdict"], "PAIR")

    def test_a_combined_file_is_one_file(self) -> None:
        self.assertEqual(search.classify(COMBINED)["verdict"], "SINGLE_FILE")
        self.assertEqual(
            search.classify({"requested_formats": [COMBINED]})["verdict"], "SINGLE_FILE"
        )

    def test_two_video_streams_are_not_a_pair(self) -> None:
        verdict = search.classify({"requested_formats": [VIDEO_ONLY, VIDEO_ONLY]})
        self.assertEqual(verdict["verdict"], "SINGLE_FILE")

    def test_offered_shapes_name_kind_protocol_and_container(self) -> None:
        shapes = search.available_shapes({"formats": [VIDEO_ONLY, AUDIO_ONLY, COMBINED, COMBINED]})
        self.assertEqual(
            shapes,
            {"video-only https mp4": 1, "audio-only https m4a": 1, "combined https mp4": 2},
        )


if __name__ == "__main__":
    unittest.main()
