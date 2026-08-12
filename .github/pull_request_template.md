<!--
Thank you for contributing.

Before requesting review:
- Read the repository AGENTS.md.
- Keep this PR in draft until all required work and validation are complete.
- Do not delete required sections.
- Check a box only after the action actually succeeded.
- If a check is not applicable or cannot be run, leave it unchecked and explain why under "Skipped checks and remaining risk".
- Replace all placeholder text. Do not submit test claims without exact results.
-->

## Change summary

<!-- Explain what changed and the user-facing outcome in 2-5 sentences. -->


## Problem and root cause

<!--
Describe the problem being solved. For a bug fix, explain the confirmed root cause rather than only the symptom.
For a feature, explain the user need and why this behavior belongs in the application.
-->


## Main changes

<!-- List the important implementation and behavior changes. Keep this scoped to the actual diff. -->

## User-visible behavior

<!--
Describe defaults, settings, error behavior, and what users will observe before and after this PR.
For a non-user-facing change, write "No user-visible behavior change" and explain why.
-->


## Compatibility and migration

<!-- Address each item. Use "Not applicable" with a reason when appropriate. -->

- Existing settings and saved data:
- Upgrade from the previous release:
- Default behavior for existing users:
- Rollback or downgrade impact:

## Testing

<!--
Thorough testing is required. Add or update focused regression tests for behavior changes.
Do not mark commands as passed unless they completed successfully on this exact branch.
-->

### Automated checks

- [ ] `npm run check`
- [ ] `npm test`
- [ ] Focused regression tests were added or updated for the changed behavior
- [ ] `npm run build:bundle` (required for installer, release, dependency, native runtime, or packaging changes)

### Command results

<!-- Include the exact result. Examples: "Passed, Rust 60 passed / 0 failed / 6 ignored" or "Not run; documentation-only change". -->

| Command | Result | Notes |
| --- | --- | --- |
| `npm run check` |  |  |
| `npm test` |  |  |
| `npm run build:bundle` |  |  |

### Change-specific coverage

<!-- Check every applicable area only after completing its required validation. -->

- [ ] Settings: defaults, persistence, malformed or missing old values, and restart behavior
- [ ] UI: Chinese and English text, control state, failure state, and real Tauri/WebView2 behavior
- [ ] Native input: keyboard, mouse, chords, unrelated held keys, repeat suppression, cancellation, and no synthetic-input recursion
- [ ] Overlay/windows: show/hide, lock/unlock, dragging, saved position, DPI scaling, and multi-monitor behavior
- [ ] OCR: region selection, display selection, capture-to-text path, matching, and failure handling
- [ ] Update/network: offline behavior, timeout, status code, response validation, size limits, and safe rendering
- [ ] Installer/upgrade: clean install, installation over the previous release, process detection, retained settings, and launch
- [ ] Performance/stability: startup, idle usage, hot path, shutdown, and bounded fallback behavior

### Test matrix

<!-- Add rows for every important scenario, including failure and compatibility paths. -->

| Area or scenario | Environment and steps | Expected result | Actual result |
| --- | --- | --- | --- |
|  |  |  |  |

### Manual runtime verification

<!--
State the real environment used, not only a mocked or parser test.
Examples: Windows version, Tauri debug/release build, WebView2, display count/DPI, input device, clean install or upgrade.
-->

- Environment:
- Steps performed:
- Observed result:

### Evidence

<!--
Attach screenshots or a short recording for visible UI changes.
Include concise log excerpts, test counts, benchmark data, or artifact checksums when relevant.
Do not include credentials, tokens, private paths, or personal data.
-->


### Skipped checks and remaining risk

<!--
List every unchecked or unavailable validation item, why it was not run, and the remaining risk.
Write "None" only when all applicable checks were completed.
-->


## Performance and stability impact

<!-- Describe startup, idle, memory, hot-path, concurrency, shutdown, and recovery impact. Do not claim gains without measurements. -->

- Startup and idle impact:
- Hot-path or concurrency impact:
- Failure and recovery behavior:
- Measurement, if claimed:

## Security, privacy, and network impact

<!-- State whether the PR changes permissions, input handling, file access, network access, update sources, or untrusted-content handling. -->


## Risks and rollback

<!-- Describe likely failure modes, affected users, how to detect a regression, and the smallest safe rollback. -->

- Risks:
- Detection:
- Rollback:

## Related issues

<!-- Use "Fixes #123" only when merging this PR should close the issue. Otherwise use "Related to #123". -->


## Final checklist

- [ ] The branch is based on the current PR target and has no unresolved conflicts
- [ ] The final diff contains only intended source, test, and documentation files
- [ ] Existing settings, loadouts, presets, custom stratagems, and overlay position remain compatible
- [ ] No secrets, private configuration, build outputs, temporary helpers, debug instrumentation, or unrelated edits are included
- [ ] User-facing Chinese and English text are both complete when applicable
- [ ] All behavior changes have focused regression coverage
- [ ] All applicable real-runtime checks are documented above
- [ ] Unverified behavior and remaining risk are stated explicitly
- [ ] The PR title and commits describe the engineering change without AI or tool branding
