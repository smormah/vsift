## Problem

The dialog, see https://example.test [c1: e1].

```vsift-handoff
{"handoff_version": "1", "status": "Partial",
 "question": "What does the dialog show after Save?",
 "capabilities": {"image_access": "verified", "image_check_code": "ABC 1234"},
 "claims": [{"id": "c1", "section": "actual", "kind": "observed", "support": "supported",
  "certainty": "high", "statement": "The dialog shows upload count 7.", "citations": ["e1"]}],
 "citations": [{"id": "e1", "type": "transcript_segment", "segment_id": "tsg_0123456789abcdef0123456789abcdef"},
  {"id": "e2", "type": "frame", "evidence_id": "evd_0123456789abcdef0123456789abcdef"}],
 "gaps": [{"kind": "image", "reason": "not_inspected", "note": null}],
 "untrusted_instructions": [],
 "lifecycle": {"action": "left_open"}}
```
