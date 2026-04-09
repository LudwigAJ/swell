# Repository Guidelines

## Project Overview

This is an autonomous coding engine project. The system is designed to autonomously handle software engineering tasks including planning, execution, testing, and validation.

For the full project specification, see `plan/Autonomous Coding Engine.md`.

## Configuration

All configurable values are in `.swell/` folder:
- `.swell/settings.json` - Runtime settings (timeouts, limits, thresholds)
- `.swell/policies/default.yaml` - Policy rules with deny-first semantics
- `.swell/models.json` - LLM model routing and configuration
- `.swell/crates.json` - Workspace crate dependencies
- `.swell/milestones.json` - Milestone definitions and blocking rules
- `.swell/prompts/` - Agent system prompts

**Never hardcode magic numbers** - always load from `.swell/settings.json` or environment variables.

## Project Structure

```
swell/
├── plan/
│   ├── Autonomous Coding Engine.md          # Master specification
│   └── research_documents/                  # Detailed subsystem specs
│       ├── Technical Architecture and Roadmap Spec.md
│       ├── Memory and Learning Architecture for an Autonomous Coding Engine.md
│       ├── Orchestrator and Execution Design Spec for an Autonomous Coding Engine.md
│       ├── Product definition and UX strategy: the autonomous engineering system.md
│       ├── Testing and Validation Research Spec for an Autonomous Coding Orchestrator.md
│       └── Tools and Runtime Control Spec for Autonomous Coding Systems.md
└── AGENTS.md                                 # This file
```

## Build, Test, and Development

- (To be added once source code is implemented)

## Coding Style & Conventions

- (To be added once source code is implemented)

## Testing Guidelines

- (To be added once source code is implemented)

## Commit & Pull Request Guidelines

- Conventional commits preferred: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`
- PRs should reference the relevant spec document in `plan/research_documents/`
- All commits must pass lint and type checks before merge

## Architecture Overview

The autonomous coding engine consists of several core subsystems:

- **Orchestrator**: Coordinates task planning and execution flow
- **Memory System**: Persists context and learned patterns across sessions
- **Tool Runtime**: Executes code, runs tests, and manages subprocesses
- **Validation Layer**: Ensures outputs meet quality and correctness standards

Detailed architecture documentation is available in `plan/Autonomous Coding Engine.md`.
