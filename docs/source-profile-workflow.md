# Source-profile workflow

The source-profile workflow has one path: `plan`, `run`, then `verify`. It never
edits the selected source project and it never copies a repository root.

## Profile contract

```json
{
  "schema": "powerbi-cli.source-profile.v1",
  "profileId": "synthetic-workbook",
  "resources": {
    "workbook": {
      "path": "data/synthetic.xlsx",
      "expectedSha256": "sha256:<64-lowercase-hex-digits>"
    }
  },
  "replacements": [
    {
      "operation": "partition.replaceSource",
      "table": "FactSales",
      "partition": "FactSales",
      "expectedBeforeSha256": "sha256:<64-lowercase-hex-digits>",
      "template": "templates/FactSales.m",
      "expectedConnector": "Excel.Workbook",
      "resources": ["workbook"]
    }
  ]
}
```

Profile, template, and resource paths are profile-relative forward-slash paths
without `..`. A resource may omit `path` only when planning supplies
`--resource workbook=<path>`. Database-only profiles may use an empty
`resources` object; every registered resource must otherwise be used by at
least one replacement and must declare its exact SHA-256. Profiles reject
unknown fields, operations, resources, placeholders, credential-like text,
absolute paths, and duplicate partition targets. Canonical profile, template,
resource, override, plan, project, and output paths are also screened for
credential-like assignment text before a plan or output is persisted.

`expectedConnector` is deliberately closed to `Excel.Workbook` and
`PostgreSQL.Database`. Planning lexes executable M rather than searching text.
It requires an actual `Source = <expected connector>(...)` root flow, rejects
unknown or dynamic function calls (including postfix calls through a computed
record, list, or parenthesized value), and rejects hard-coded file and URI paths.
An Excel root must be exactly one
`Excel.Workbook(File.Contents("<declared resource placeholder>"), ...)` flow;
a PostgreSQL root takes no file resources.

The M template is the complete partition expression. It must execute the
expected connector and contain exactly the declared placeholders:

```powerquery
let
    Source = Excel.Workbook(
        File.Contents("{{powerbi-cli.resourcePath:workbook}}"),
        null,
        true
    ),
    Navigation = Source{[Item="FactSales", Kind="Table"]}[Data],
    Typed = Table.TransformColumnTypes(Navigation, {{"Amount", Currency.Type}})
in
    Typed
```

## Commands

```bash
powerbi-cli workflow plan --project Report.pbip --profile workflow/source-profile.json --out ../powerbi-build/report.plan.json --out-dir ../powerbi-build/report --json
powerbi-cli workflow run --plan ../powerbi-build/report.plan.json --confirm sha256:<plan-fingerprint> --json
powerbi-cli workflow verify --plan ../powerbi-build/report.plan.json --json
```

`plan` writes only a new plan file. Identical inputs and intended output produce
the same fingerprint. `run` rechecks the plan and every input before creating a
new output, copies only the selected PBIP/Report/SemanticModel closure and named
resources, applies exact partition source ranges through the pinned local MCP
process, and requires strict native plus official report validation. Its receipt
contains hashes, exact component versions, counts, cleanup evidence, and the
quarantined model-export path/hash; it contains no M payload, query rows,
credentials, or images.

The newly created output directory is opened once as a filesystem capability.
Directory creation, create-only file writes, copy readback, and marker removal
are relative to that open handle. Each intermediate component is opened without
following a link, and the final filename uses atomic create-new semantics. A
rename or junction/symlink swap at the ambient pathname therefore cannot
redirect a workflow write; the final path/FileId publication check still fails
if the output pathname no longer names the opened directory.

The plan file and intended output must both be outside the entire source
project root. That containment is rechecked from canonical paths whenever a
plan is loaded, so recomputing a plan fingerprint cannot authorize a write
below the source project. The selected closure is an allowlist: the PBIP entry;
PBIR `definition.pbir`, JSON definition files, and registered report assets;
and SemanticModel PBISM, TMDL, diagram, and platform metadata. Private/cache
directories, `localSettings.json`, Power BI caches/binaries, unregistered data
files, links/reparse points, and credential-bearing source text are rejected.

`verify` is read-only. It reconstructs the plan's templates, replacements, and
resource bindings from the current profile; derives the expected staged TMDL
tree; checks every unmodified closure file and resource against the plan; binds
the exported partition readbacks to the receipt; and recomputes both validation
results. It also asks the exact local modeling MCP to perform a fresh read-only
canonical export of the complete staged model into a private OS temporary
directory. The fresh definition hash, file count, and byte count must equal the
receipt evidence, so adding unrelated valid TMDL cannot be hidden by resealing
self-hashes. Any mismatch fails even if a self-hash was recomputed. Every
evidence TMDL file and every SVG resource is bounded UTF-8 text and receives the
credential scan. Tree limits are enforced from metadata before a file is opened
and again while streaming, so an oversized file is never read to EOF. An output containing
`.powerbi-cli-workflow-incomplete` is retained only for diagnosis and must not
be published.

The receipt checksum detects accidental edits; it is not a signature. Verify
also checks claims against current state: exact installed sidecar versions,
successful child cleanup, byte-identical source evidence, expected stage hash,
exact partition readback, output/evidence hashes, and both validator results.
Recomputing a checksum cannot make inconsistent claims pass.
