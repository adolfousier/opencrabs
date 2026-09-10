# Plan JSON Schema Reference

## Minimal Import Format

Only **3 fields required** at the root and **3 per task** for a valid import:

### Root Level (3 fields)
```json
{
  "title": "Plan name",
  "description": "What this plan accomplishes",
  "tasks": [...]
}
```

### Task Level (3 fields per task)
```json
{
  "title": "Task name",
  "description": "What this task does",
  "task_type": "research"
}
```

## Optional Task Fields

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `dependencies` | `integer[]` or `string[]` | *(omitted)* | **Optional.** Omit when there are no dependencies — empty `[]` is redundant. When present, use 1-based integer indices (e.g. `[1, 2]`) for readability. UUID strings also accepted. |
| `complexity` | `number` | `3` | 1-5 scale: 1=trivial, 2=simple, 3=moderate, 4=complex, 5=very complex |
| `acceptance_criteria` | `string[]` | `[]` | List of conditions that mark this task complete |

## Optional Plan Field

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `working_directory` | `string` | *(omitted)* | **Optional (#1452).** Absolute path to the repo the plan's work executes in. Receipt binding (#1011) verifies claimed commit shas against this repo instead of the session cwd, so cross-repo plans (session in repo A, work in repo B) can carry mechanical commit receipts. Must exist and be absolute; validated on import and on the `init` param (an explicit `init` param overrides a JSON-carried value). Omit for plans executed in the session's own repo. |

## Auto-Generated Fields (Do NOT Provide)

These are always overwritten on import:

### Root Level
- `id` — UUID, regenerated
- `session_id` — UUID, from session
- `status` — set on write: `"Editing"` on `plan init`, `"Active"` on Approve / `/execute` (never `"Draft"`)
- `created_at` — ISO timestamp
- `updated_at` — ISO timestamp  
- `approved_at` — `null` until user Approve on the design track, then an ISO timestamp

### Task Level
- `id` — UUID, auto-minted (do not provide — see Optional Fields note above)
- `order` — 1-based; auto-assigned from array position if omitted (recommended to omit)
- `status` — always `"Pending"`
- `notes` — always `null`
- `retry_count` — always `0`

Legacy fields (`context`, `risks`, `technical_stack`, `test_strategy` at plan level;
`completed_at`, `max_retries`, `artifacts` at task level) were removed
from the schema. They are ignored on import of old plan JSON files.

## task_type Values

Case-insensitive. Examples: `"Research"`, `"research"`, `"RESEARCH"` all work.

| Value | Description |
|-------|-------------|
| `research` | Investigate, explore, understand |
| `edit` | Modify existing code/files |
| `create` | Build new things from scratch |
| `delete` | Remove code/files |
| `test` | Write or run tests |
| `refactor` | Restructure existing code without changing behavior |
| `documentation` | Docs, comments, specs |
| `configuration` | Config, setup, infrastructure |
| `build` | Compile, package, release |
| `other` | Anything that fits no category above (also the default when omitted) |

Fallback: a value outside this table is not rejected. It is preserved verbatim and categorized as `other`.

## Dependencies

Use **1-based integer indices** (human-friendly). **Omit the field when there are no dependencies** — do not write `"dependencies": []`.

```json
"dependencies": [1, 2]        // depends on tasks 1 and 2
"dependencies": [1]            // depends on task 1 only
// (no field at all)           // no dependencies — preferred over []
```

## JSON Schema (Machine-Readable)

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "type": "object",
  "required": ["title", "description", "tasks"],
  "additionalProperties": true,
  "properties": {
    "title": { "type": "string" },
    "description": { "type": "string" },
    "tasks": {
      "type": "array",
      "minItems": 1,
      "items": {
        "type": "object",
        "required": ["title", "description", "task_type"],
        "additionalProperties": true,
        "properties": {
          "title": { "type": "string" },
          "description": { "type": "string" },
          "task_type": { 
            "type": "string",
            "enum": ["research", "edit", "create", "delete", "test", "documentation", "configuration"],
            "pattern": "^(?i)(research|edit|create|delete|test|documentation|configuration)$"
          },
          "dependencies": {
            "type": "array",
            "items": { "oneOf": [{ "type": "integer", "minimum": 1 }, { "type": "string", "pattern": "^[0-9a-fA-F-]{36}$" }] },
            "default": []
          },
          "complexity": { "type": "integer", "minimum": 1, "maximum": 5, "default": 3 },
          "acceptance_criteria": { "type": "array", "items": { "type": "string" }, "default": [] }
        }
      }
    }
  }
}
```

## Example Minimal Plan

```json
{
  "title": "Add user authentication",
  "description": "Implement login/logout flow with session management",
  "tasks": [
    { "title": "Research auth patterns", "description": "Look at existing auth code and pick a pattern", "task_type": "research" },
    { "title": "Write login handler", "description": "POST /auth/login with password verification", "task_type": "create", "dependencies": [1] },
    { "title": "Add session middleware", "description": "Attach user context to requests", "task_type": "configuration", "dependencies": [2] },
    { "title": "Write auth tests", "description": "Test login, logout, and session expiry", "task_type": "test", "dependencies": [3], "complexity": 2 }
  ]
}
```

## Files

- **Minimal example**: `~/.opencrabs/profiles/ops/plans/coding-plans/sample-minimal-plan.json`
- **Full example**: `~/.opencrabs/profiles/ops/plans/coding-plans/rust-full.json`
