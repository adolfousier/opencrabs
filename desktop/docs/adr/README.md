# Architecture Decision Records

This directory records architecturally significant decisions for the OpenCrabs
desktop app, so the *why* survives the people who made it.

| ADR | Title | Status |
|-----|-------|--------|
| [0001](0001-build-dioxus-frontend-with-dx-cli.md) | Build the Dioxus frontend with the `dx` CLI, not Trunk | Accepted |
| [0002](0002-native-frontend-mount-verification.md) | Verify desktop releases with a reproducible gate ladder + native smoke | Accepted |

## Format

Each ADR follows a short template: Status, Context, Decision, Consequences,
Alternatives considered. New decisions append the next number; superseded
decisions are marked and linked to their replacement rather than deleted.
