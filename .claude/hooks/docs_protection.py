#!/usr/bin/env python3
"""
RAIL — /docs/ protection hook for Claude Code.

This hook runs before any write operation to files in /docs/.
It enforces documentation rules defined in CLAUDE.md and /docs/README.md.

Rules enforced:
1. Topic already covered in another file? Block creation of duplicates.
2. New file not listed in /docs/README.md? Block until it is added.
3. File exceeds ~150 lines without a table of contents? Warn.
4. /docs/README.md itself exceeds 200 lines? Block.
5. Math content duplicating DSP.md or SIGNALS.md? Warn.

Hook type: PreToolUse (runs before Write, Edit, MultiEdit)
"""

import json
import sys
import os
import re

def load_readme_index(docs_path):
    readme_path = os.path.join(docs_path, "README.md")
    if not os.path.exists(readme_path):
        return []
    with open(readme_path, "r") as f:
        content = f.read()
    # Extract filenames from the index table rows
    filenames = re.findall(r"`([A-Z_]+\.md)`", content)
    return filenames

def count_lines(filepath):
    if not os.path.exists(filepath):
        return 0
    with open(filepath, "r") as f:
        return sum(1 for _ in f)

def has_table_of_contents(filepath):
    if not os.path.exists(filepath):
        return False
    with open(filepath, "r") as f:
        content = f.read(3000)  # Check first 3000 chars only
    return "## Table of contents" in content or "# Table of contents" in content

def main():
    try:
        hook_input = json.load(sys.stdin)
    except json.JSONDecodeError:
        sys.exit(0)  # Not valid JSON — let Claude Code handle it

    tool_name = hook_input.get("tool_name", "")
    tool_input = hook_input.get("tool_input", {})

    # Only care about write/edit operations
    if tool_name not in ("Write", "Edit", "MultiEdit", "create_file", "str_replace"):
        sys.exit(0)

    # Get the target file path
    file_path = tool_input.get("path") or tool_input.get("file_path", "")
    if not file_path:
        sys.exit(0)

    # Only intercept files in /docs/
    if "/docs/" not in file_path and "\\docs\\" not in file_path:
        sys.exit(0)

    # Locate /docs/ directory
    docs_path = None
    for part in file_path.replace("\\", "/").split("/"):
        pass  # walk to find docs root
    # Simple: find docs/ in the path
    path_parts = file_path.replace("\\", "/").split("/")
    for i, part in enumerate(path_parts):
        if part == "docs":
            docs_path = "/".join(path_parts[:i+1])
            break

    if not docs_path:
        sys.exit(0)

    filename = os.path.basename(file_path)
    is_readme = filename == "README.md"
    issues = []
    warnings = []

    # --- Rule 1: Check README.md line count ---
    if is_readme:
        # Estimate new line count from content being written
        new_content = tool_input.get("content") or tool_input.get("new_str") or ""
        new_lines = len(new_content.splitlines())
        existing_lines = count_lines(file_path) if os.path.exists(file_path) else 0
        # For edits, approximate
        if new_lines > 200 or existing_lines > 200:
            issues.append(
                f"BLOCKED: /docs/README.md must stay under 200 lines. "
                f"Estimated size after edit: {max(new_lines, existing_lines)} lines. "
                f"README is an index only — move content to a dedicated doc file."
            )

    # --- Rule 2: New file must be in README index ---
    if not is_readme and not os.path.exists(file_path):
        # It's a new file — check if it's in the README
        indexed_files = load_readme_index(docs_path)
        if filename not in [f"{n}.md" for n in []] and filename not in indexed_files:
            issues.append(
                f"BLOCKED: '{filename}' is not listed in /docs/README.md. "
                f"Add it to the README index table before creating this file. "
                f"Check if an existing doc already covers this topic first."
            )

    # --- Rule 3: Long files need table of contents ---
    if not is_readme and os.path.exists(file_path):
        line_count = count_lines(file_path)
        if line_count > 150 and not has_table_of_contents(file_path):
            warnings.append(
                f"WARNING: '{filename}' has {line_count} lines but no table of contents. "
                f"Add '## Table of contents' section at the top (max 50 lines)."
            )

    # --- Rule 4: Remind about documentation rules ---
    reminder = """
DOCUMENTATION RULES (enforced — read before proceeding):
─────────────────────────────────────────────────────────
1. Does an existing /docs/ file already cover this topic?
   → If yes: add there. Do NOT create a new file.

2. Is this file listed in /docs/README.md?
   → If no: add it to README.md FIRST.

3. /docs/README.md must stay under 200 lines (index only).

4. Files longer than ~150 lines must start with a table of contents.

5. Never duplicate math from DSP.md — reference it instead.

6. Backend code: reference docs, do not re-explain math inline.

7. Frontend docs: minimal. Code should be self-explanatory.

Topic ownership quick reference:
  DSP math, FFT, demodulation → DSP.md
  IPC, threading, modules → ARCHITECTURE.md
  RTL-SDR hardware, librtlsdr → HARDWARE.md
  SigMF, signal types, bands → SIGNALS.md
  Naming, error handling, style → CONVENTIONS.md
  Build phases, milestones → TIMELINE.md
─────────────────────────────────────────────────────────
"""

    # Output result
    if issues:
        output = {
            "decision": "block",
            "reason": "\n".join(issues) + "\n" + reminder
        }
        print(json.dumps(output))
        sys.exit(0)

    if warnings:
        output = {
            "decision": "warn",
            "reason": "\n".join(warnings) + "\n" + reminder
        }
        print(json.dumps(output))
        sys.exit(0)

    # No issues — but still print the reminder so Claude Code sees it
    output = {
        "decision": "allow",
        "reason": reminder
    }
    print(json.dumps(output))
    sys.exit(0)

if __name__ == "__main__":
    main()
