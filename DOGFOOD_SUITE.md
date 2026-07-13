# Maintained dogfood suite

Fixture contract for `pi-flake` `94694da7321ce74aa7b82c13db7e60e28c0caba6` (Pi 0.80.6).
The source runtime is an extension-behavior oracle only; pi-rs product parity remains pinned to Pi v0.79.0.

Normal checks consume `tests/dogfood-suite/contract.json` and do not require a sibling checkout.
`scripts/dogfood-oracle --source /path/to/pi-flake --check` additionally verifies the pinned source trees and package declarations.

| Package | Upstream source | Version | Default bundle | Fixture cases | Cleanup assertions |
|---|---|---:|:---:|---:|---|
| `codex-fast` | `pi-codex-fast/src/index.ts` | 1.0.0 | yes | 1 | status key removed on replacement/reload; no state survives session shutdown |
| `gecko-websearch` | `pi-gecko-websearch/src/index.ts` | 1.0.0 | yes | 1 | session shutdown closes every Marionette socket; browser processes are reaped; temporary profiles are removed |
| `rtk` | `pi-rtk/index.ts` | 0.3.0 | yes | 1 | rewrite subprocesses are killed on timeout/reload; replacement bash execution owns no process after shutdown |
| `compact` | `pi-compact/src/index.ts` | 0.1.0 | yes | 1 | renderer middleware registrations are disposed on reload; render timer is cancelled on shutdown |
| `context-janitor` | `pi-context-janitor/src/index.ts` | 0.1.0 | yes | 1 | sidecar timer and task are cancelled before session replacement; status and custom renderer are removed on reload; old-session index handles reject writes |
| `morph` | `pi-morph/src/index.ts` | 0.1.0 | no | 1 | in-flight gateway request aborts on reload; per-file mutation ownership releases on error/shutdown |
| `tool-management` | `pi-tool-management/src/index.ts` | 1.0.0 | yes | 1 | custom settings overlay is disposed on session switch; active-tool filter is recomputed after reload |
| `webfetch` | `pi-webfetch/src/index.ts` | 1.0.0 | yes | 1 | in-flight requests abort with the tool signal; LRU cache is disposed on package replacement |
| `hashline` | `pi-hashline/src/index.ts` | 0.2.0 | yes | 1 | temporary atomic-write files are removed after success/error; per-file mutation queue entries are released on shutdown |
| `minimal-editor` | `pi-minimal-editor/src/index.ts` | 0.1.0 | yes | 1 | branch-change subscription is disposed; editor and footer slots are cleared on session shutdown/replacement |
| `working-indicator` | `pi-working-indicator/extensions/index.ts` | 0.1.0 | yes | 1 | startup timeout is cancelled on model/session replacement; working message and indicator slots are cleared on shutdown |
| `pomodoro` | `pi-pomodoro/src/index.ts` | 1.0.0 | yes | 1 | interval and file watcher are disposed on reload/session replacement; status key is removed on shutdown; no timer writes after disposal |
| `rlm` | `pi-rlm/src/index.ts` | 0.1.0 | yes | 1 | root/child Python REPL process trees are killed on abort/reload/shutdown; child provider tasks are cancelled before session replacement; context stores are generation-bound |
| `review` | `earendil_pi-review/review.ts` | 0.1.0 | yes | 1 | git/gh processes and custom overlays abort on reload; review widget/editor state is cleared on end-review and session replacement; stale review state does not cross branches |
| `vcc` | `sting8k_pi-vcc/index.ts` | 0.3.12 | yes | 1 | compaction/recall tasks are cancelled on session replacement; session-derived caches are rebuilt after branch/reload; temporary report files are removed |

## Deterministic fixture coverage

| Case | Kinds | Scripted boundary | Expected observation |
|---|---|---|---|
| `codex-fast.priority-request` | provider, filesystem, terminal | global_config, project_config, model, request | request_patch, status, command_state |
| `gecko-websearch.search-and-browse` | browser_socket, subprocess, filesystem | profile, marionette, search_html, browse_text | tools, search_result_count, browse_content, progress |
| `rtk.rewrite-fallback` | subprocess | commands, rtk, user_bash_excluded | agent_commands, excluded_user_bash_claimed, rewrite_timeout_ms |
| `compact.middleware-modes` | terminal, timer | settings, rows, clock_ms | tool_row, thinking_row, expanded_uses_default, private_class_patches |
| `context-janitor.clean-undo-branch` | provider, compaction, session, timer, filesystem, terminal | tool_results, raw_chars, sidecar_decision, session_changes, clock_ms | cleaned_results, context_replacement, notice_custom_type, undo_restores_raw, history_is_branch_scoped |
| `morph.apply-response` | provider, filesystem, terminal | file, instruction, gateway, model | request_contains_original, file, tool, status_cleared |
| `tool-management.persist-filter` | filesystem, terminal, session | tools, active, settings, toggle | active, blocked_external, persisted_disabled, unknown_names_retained |
| `webfetch.redirect-markdown-cache` | provider, timer | requests, clock_ms, prompt | https_first, same_host_redirect_limit, markdown_fixture, second_result_cached, expired_result_refetched |
| `hashline.read-edit-atomic` | filesystem, session | file, read, edit, race | read_format, edited_file, stale_anchor_fails_closed, symlink_and_mode_preserved |
| `minimal-editor.footer-frame` | terminal, session | widths, cwd, branch, session_name, usage | padding_x, top_contains, bottom_contains, cursor_preserved |
| `working-indicator.seeded-frames` | terminal, timer | random, theme, thinking, ticks_ms | width, interval_ms, startup_switch_ms, working_message, frame_fixture |
| `pomodoro.synced-lifecycle` | timer, filesystem, terminal, session | settings, commands, clock_ms, external_sync | phases, state_write_is_atomic, transition_notification, tick_ms |
| `rlm.recursive-finalization` | provider, subprocess, session, terminal | root_prompt, repl, child_provider_outputs, max_concurrent, max_depth, session_changes | child_tools, completion_order_preserved, custom_type, final_not_refed_to_model |
| `review.local-and-pr-workflow` | subprocess, filesystem, session, terminal, compaction | git, gh, commands, session_changes | widget, system_prompt_has_review_instructions, append_entry_tracks_review, editor_restored |
| `vcc.compact-recall-lineage` | compaction, session, filesystem, terminal, timer | jsonl_fixture, commands, scope, session_changes, clock_ms | sections, recall_filters_noise, lineage_excludes_off_branch, before_compact_result_is_stable |

Load modes pinned for every package: direct file, configured package, and Nix bundle.
The default source bundle contains 14 packages and excludes opt-in `morph`; the all-package acceptance bundle contains all 15.

This fixture-only preflight intentionally contains no inert translation packages: executable Lua translations depend on the public lifecycle/module/mechanism work owned by PLAN 9.2–9.9.
