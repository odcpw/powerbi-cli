# Input-Surface Safety Contract

Date: 2026-09-04

Every file that enters through a CLI argument is untrusted. Readers must use
the typed guards in `src/input_safety.rs`, reject before parsing or applying
when a limit is exceeded, decode text strictly as UTF-8, and return
`error.code = "input_safety_violation"` (exit 10) with a hint and executable
`suggestedCommands[]`. Schema/spec composition additionally maps the guard's
path and cycle refusals to the stable, more actionable `include.path_escape`
and `include.cycle` diagnostic codes (still exit 10); budget, symlink, and
other input-boundary failures retain `input_safety_violation`. Limits are
fixed contract values, not automatically raised from file metadata or content.

The same values are machine-readable at `capabilities.limits`, including in a
focused `capabilities --for ...` response.

| Surface | Limit and policy |
|---|---|
| Schema | 8 MiB; ordinary non-symlink UTF-8 file |
| Data profile | 8 MiB; ordinary non-symlink UTF-8 file |
| Dashboard spec | 8 MiB; ordinary non-symlink UTF-8 file |
| Other JSON artifact | 16 MiB; ordinary non-symlink UTF-8 file |
| Project PBIP/PBIR/TMDL text | 16 MiB per file; ordinary non-symlink UTF-8 file |
| DAX, textbox, and similar source text | 2 MiB; strict UTF-8; NUL refused |
| `$include` fragment | 8 MiB each; relative-only; no `..`; canonical root containment; symlinks/reparse points and cycles refused; depth at most 8; at most 200 resolved fragments, excluding the root document |
| CSV/JSON rows | 64 MiB; at most 100,000 logical records (CSV header included) and 512 fields in any record; strict decoding; malformed CSV/JSON refused; leading `=`, `+`, `-`, and `@` remain exact text and are never rewritten |
| Intent | 1 MiB; strict UTF-8; line directives beginning with `$`, `@`, `!`, or `#` plus `include`/`exec`, and `include:`/`exec:`, are refused for file and inline intent |
| Image | PNG only in v1; 16 MiB; the eight-byte PNG signature is sniffed, so an extension cannot authorize content; URL inputs are not accepted |
| Ops file | 8 MiB; exact `powerbi-cli.ops.v1` schema marker and typed op-kind allowlist must pass before apply; unknown op kinds are refused |
| Snapshot | Sibling path by default or explicit `--snapshot-dir`; source at most 10,000 files and 512 MiB; links, an inside-project destination, an existing destination, and an unwritable destination are refused |
| Harvested PBIR fragment | 4 MiB; known persisted selection/filter value containers (including Desktop slicer `filter/.../Value` comparisons) are refused with a JSON pointer; the guard never silently strips content |

Package archives and deterministic staged-workflow resources retain their
stricter specialized streaming limits and file-identity checks. This contract
does not weaken or replace those boundaries.

## API ownership for planned surfaces

- Schema/spec composition calls `IncludeGuard::new` once for the root and
  `IncludeGuard::resolve` for every edge while passing the active canonical
  stack. The guard owns depth, total-count, traversal, containment, symlink,
  and cycle refusals; the composition layer preserves the semantic
  `include.path_escape`/`include.cycle` codes and includes the active chain for
  cycles.
- Profile v2 calls `read_rows`; its returned `BoundedRows` contains exact CSV
  strings or the parsed JSON value plus observed row/column counts.
- Registered-resource image authoring calls `read_png` before it registers or
  writes any resource.
- Batch apply calls `read_ops` with its complete typed op-kind catalog before
  deserializing or starting a transaction.
- In-place batch/sanitize work calls `snapshot_destination` before copying.
- The Desktop reference harvester calls `read_harvested_fragment`; persisted
  data is a refusal and must be removed explicitly in the source workflow.

None of these APIs advertises an unimplemented command. The command-owning
bead wires its surface to this contract when that command becomes real.
