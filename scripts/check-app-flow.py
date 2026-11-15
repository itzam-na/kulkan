#!/usr/bin/env python3
"""Check that the app-flow guide still points at real code and decisions."""

from __future__ import annotations

import re
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
GUIDE = ROOT / "docs/app-flow.md"
SPEC = ROOT / "SPEC.md"

# Stable behavior seams named in docs/app-flow.md. Prefer public boundaries and
# responsibilities over line numbers, which change routinely during edits.
SOURCE_CONTRACTS = {
    "src/main.rs": [
        "fn run(",
        "fn build_main_window(",
        "fn install_library_events(",
        "fn execute_workspace_effects(",
        "fn perform_playback_request(",
        "fn start_catalog_scan(",
        "fn install_playback_poll(",
    ],
    "src/library_workspace.rs": [
        "pub(crate) struct LibraryWorkspace",
        "pub(crate) fn dispatch(",
        "fn complete_search(",
    ],
    "src/search_state.rs": ["pub(crate) struct SearchWorkspace"],
    "src/catalog_selection.rs": [
        "pub struct SelectionState",
        "pub fn select(",
    ],
    "crates/kulkan-runtime/src/lib.rs": [
        "pub fn serve(",
        "struct LibraryService",
        "pub struct ServerProcess",
        "pub struct ServerClient",
        "fn scan_roots(",
    ],
    "crates/kulkan-runtime/src/queue.rs": [
        "pub(super) struct QueueService",
        "fn observe_snapshot(",
        "fn advance(",
        "fn start_entry(",
    ],
    "crates/kulkan-runtime/src/playback.rs": [
        "pub(super) struct PlaybackService",
        "fn play_with_state(",
    ],
    "crates/kulkan-runtime/src/playback_attempt.rs": [
        "pub(super) struct PlaybackAttempt",
        "fn should_qualify(",
    ],
    "crates/kulkan-library/src/lib.rs": [
        "pub fn scan_library_selected(",
        "fn read_metadata(",
        "fn content_fingerprint(",
        "fn derive_duration(",
    ],
    "crates/kulkan-library/src/browse.rs": ["pub fn browse_catalog("],
    "crates/kulkan-library/src/search.rs": ["pub fn search_catalog("],
    "crates/kulkan-ipc/src/lib.rs": [
        "pub enum Message",
        "pub fn bind_private_socket(",
        "pub fn write_message<",
        "pub fn read_message<",
    ],
    "crates/kulkan-runtime/src/bin/kulkan-server.rs": ["fn main("],
}

REQUIRED_HEADINGS = [
    "## Repository map: where responsibilities live",
    "## Process startup, ownership, and shutdown",
    "## Library lifecycle: choose, scan, reconcile, present",
    "## Browse and Search: user action to visible page",
    "## Queue and playback",
    "## State: what survives and where",
    "## Decisions and implementation traceability",
    "## Tests and contributor workflow",
]

DECISION_ISSUES = [*range(2, 18), 21]


def fail(errors: list[str], message: str) -> None:
    errors.append(message)


def main() -> int:
    errors: list[str] = []
    guide_text = GUIDE.read_text(encoding="utf-8") if GUIDE.exists() else ""
    spec_text = SPEC.read_text(encoding="utf-8") if SPEC.exists() else ""

    if not guide_text:
        fail(errors, "missing or empty docs/app-flow.md")
    if not spec_text:
        fail(errors, "missing or empty SPEC.md")

    for heading in REQUIRED_HEADINGS:
        if heading not in guide_text:
            fail(errors, f"guide is missing required section: {heading}")

    # Resolve local Markdown links, catching moved/removed code, ADR, and docs.
    for target in re.findall(r"\[[^\]]*\]\(([^)]+)\)", guide_text):
        if target.startswith(("https://", "http://", "mailto:", "#")):
            continue
        relative_path = target.split("#", 1)[0]
        if not relative_path:
            continue
        resolved = (GUIDE.parent / relative_path).resolve()
        if not resolved.is_file():
            fail(errors, f"broken local link in guide: {target}")

    for source, markers in SOURCE_CONTRACTS.items():
        source_path = ROOT / source
        if not source_path.is_file():
            fail(errors, f"missing source file cited by the guide: {source}")
            continue
        source_text = source_path.read_text(encoding="utf-8")
        link = f"](../{source})"
        if link not in guide_text:
            fail(errors, f"guide no longer links to behavior source: {source}")
        for marker in markers:
            if marker not in source_text:
                fail(errors, f"source seam moved/renamed: {source} no longer has {marker!r}")
            # Ensure the specific symbols being checked are actually named in
            # the contributor map, rather than silently orphaning the contract.
            symbol = marker.split("(", 1)[0].split()[-1].rstrip("<")
            symbol = symbol.split("::")[-1]
            if symbol not in guide_text:
                fail(errors, f"guide does not mention source seam {source}::{symbol}")

    for issue in DECISION_ISSUES:
        issue_url = f"issues/{issue}"
        if issue_url not in spec_text:
            fail(errors, f"SPEC.md no longer links decision issue #{issue}")
        if f"#{issue}" not in guide_text:
            fail(errors, f"guide no longer maps decision issue #{issue}")

    for required in ["GLOSSARY.md", "SPEC.md", "Wayfinder decision map", "ADR-0001"]:
        if required not in guide_text:
            fail(errors, f"guide is missing decision/domain reference: {required}")

    if errors:
        print("App-flow documentation check failed:", file=sys.stderr)
        for error in errors:
            print(f"- {error}", file=sys.stderr)
        return 1

    print(
        f"App-flow documentation check passed: {len(SOURCE_CONTRACTS)} source files, "
        f"{sum(map(len, SOURCE_CONTRACTS.values()))} source seams, and "
        f"{len(DECISION_ISSUES)} decision links verified."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
