#!/usr/bin/env bash
#
# End-to-end smoke test that exercises the real daemon binary and CLI
# binary together — not the in-process Daemon::run() like the cargo
# integration test does. Catches packaging-level regressions: arg
# parsing, socket env var defaults, dashboard port collisions, the
# CLI's confirm() flow, and (most importantly) command handlers that
# emit the right event but forget to mutate orchestrator state.
#
# Usage:  scripts/smoke_test.sh
# Exits 0 on success, non-zero on the first failed assertion.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DAEMON_BIN="${REPO_ROOT}/target/debug/swell-daemon"
CLI_BIN="${REPO_ROOT}/target/debug/swell"

WORK_DIR="$(mktemp -d -t swell-smoke-XXXXXX)"
SOCKET="${WORK_DIR}/daemon.sock"
DAEMON_LOG="${WORK_DIR}/daemon.log"
DAEMON_PID=""

cleanup() {
  if [[ -n "${DAEMON_PID}" ]] && kill -0 "${DAEMON_PID}" 2>/dev/null; then
    kill "${DAEMON_PID}" 2>/dev/null || true
    wait "${DAEMON_PID}" 2>/dev/null || true
  fi
  rm -rf "${WORK_DIR}"
}
trap cleanup EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
ok()   { echo "ok: $*"; }

# Build only what we need. Skip if already built.
if [[ ! -x "${DAEMON_BIN}" ]] || [[ ! -x "${CLI_BIN}" ]]; then
  echo "building swell-daemon and swell..."
  (cd "${REPO_ROOT}" && cargo build --bin swell-daemon --bin swell)
fi

# Run the daemon out of WORK_DIR so its SQLite memory store lands in
# WORK_DIR/.swell/memory.db rather than the repo. Use a per-run
# dashboard port so two concurrent smoke runs don't collide.
cd "${WORK_DIR}"
mkdir -p "${WORK_DIR}/.swell"
DASHBOARD_PORT=$((30000 + (RANDOM % 30000)))
SWELL_SOCKET="${SOCKET}" SWELL_DASHBOARD_PORT="${DASHBOARD_PORT}" \
  "${DAEMON_BIN}" >"${DAEMON_LOG}" 2>&1 &
DAEMON_PID=$!

# Wait for socket.
for _ in $(seq 1 50); do
  [[ -S "${SOCKET}" ]] && break
  sleep 0.1
done
[[ -S "${SOCKET}" ]] || { tail -50 "${DAEMON_LOG}"; fail "daemon never bound socket"; }
ok "daemon listening on ${SOCKET}"

export SWELL_SOCKET="${SOCKET}"

# 1. Status round-trip.
"${CLI_BIN}" status >/dev/null || fail "swell status failed"
ok "status round-trips"

# 2. Create task, list, parse JSON.
"${CLI_BIN}" task "smoke task" | grep -q "Task created:" || fail "task create"
TASK_JSON="$("${CLI_BIN}" list --json)"
TASK_ID="$(echo "${TASK_JSON}" | python3 -c "import json,sys; print(json.load(sys.stdin)[0]['id'])")"
[[ "${TASK_ID}" =~ ^[0-9a-f-]{36}$ ]] || fail "list returned bad task id: ${TASK_ID}"
ok "task created (${TASK_ID})"

STATE_BEFORE="$(echo "${TASK_JSON}" | python3 -c "import json,sys; print(json.load(sys.stdin)[0]['state'])")"
[[ "${STATE_BEFORE}" == "CREATED" ]] || fail "expected CREATED, got ${STATE_BEFORE}"
ok "list reports state=CREATED"

# 3. Cancel must persist (regression for the bug found 2026-04-26:
#    handler emitted the state-changed event without actually calling
#    fail_task on the orchestrator, so list kept showing CREATED).
echo "y" | "${CLI_BIN}" cancel "${TASK_ID}" >/dev/null
sleep 0.2
STATE_AFTER="$("${CLI_BIN}" list --json | python3 -c "import json,sys; print(json.load(sys.stdin)[0]['state'])")"
[[ "${STATE_AFTER}" == "FAILED" ]] || fail "cancel did not persist; state=${STATE_AFTER}"
ok "cancel persists state=FAILED"

# 4. Status reflects new tasks_by_state counts.
TASKS_FAILED="$("${CLI_BIN}" status | grep -E '^\s+Failed:' | awk '{print $2}')"
[[ "${TASKS_FAILED}" == "1" ]] || fail "status didn't reflect cancelled task; saw '${TASKS_FAILED}'"
ok "status counts cancelled task as Failed"

# 5. Multi-agent pipeline handoff: `swell execute` must drive the task
#    through PlannerAgent → GeneratorAgent → EvaluatorAgent. We assert
#    on the daemon log lines the agents emit, not on terminal task
#    state — under MockLlm the Evaluator stage cannot satisfy the
#    state-machine precondition, so the task ends Failed; what we want
#    to prove here is that handoff *occurred*, which closes the wiring
#    gap where `execute_task` was unreachable from the CLI surface.
"${CLI_BIN}" task "pipeline handoff smoke" >/dev/null
PIPE_TID="$("${CLI_BIN}" list --json | python3 -c "import json,sys; tasks=json.load(sys.stdin); print(next(t['id'] for t in tasks if t['description']=='pipeline handoff smoke'))")"
[[ "${PIPE_TID}" =~ ^[0-9a-f-]{36}$ ]] || fail "could not find pipeline-handoff task id"
"${CLI_BIN}" execute "${PIPE_TID}" >/dev/null || fail "swell execute failed"

# The tracing subscriber writes ANSI color codes around field names
# (e.g. `\x1b[3magent\x1b[0m=Planner`), so a literal `grep agent=Planner`
# never matches. Strip ANSI escapes once, then assert on the
# human-readable message bodies the agents emit at each handoff.
ANSI_STRIP='s/\x1b\[[0-9;]*[a-zA-Z]//g'
clean_log() { sed "${ANSI_STRIP}" "${DAEMON_LOG}"; }

# Wait up to 5s for the spawned pipeline to log all three agent stages.
for _ in $(seq 1 50); do
  CLEAN="$(clean_log)"
  if   echo "${CLEAN}" | grep -q "Starting plan generation" \
    && echo "${CLEAN}" | grep -q "handing off to Generator" \
    && echo "${CLEAN}" | grep -q "Starting code generation" \
    && echo "${CLEAN}" | grep -q "handing off .* to Evaluator"; then
    break
  fi
  sleep 0.1
done

CLEAN="$(clean_log)"
echo "${CLEAN}" | grep -q "Starting plan generation" \
  || { echo "${CLEAN}" | tail -60; fail "PlannerAgent did not start"; }
ok "PlannerAgent ran"

echo "${CLEAN}" | grep -q "handing off to Generator" \
  || { echo "${CLEAN}" | tail -60; fail "Planner did not hand off to Generator"; }
ok "Planner → Generator handoff"

echo "${CLEAN}" | grep -q "Starting code generation" \
  || { echo "${CLEAN}" | tail -60; fail "GeneratorAgent did not start"; }
ok "GeneratorAgent ran"

echo "${CLEAN}" | grep -q "handing off .* to Evaluator" \
  || { echo "${CLEAN}" | tail -60; fail "Generator did not hand off to Evaluator"; }
ok "Generator → Evaluator handoff"

# Pipeline must reach a terminal state within a few seconds (Failed is
# expected under MockLlm; success or failure both close the wiring loop).
PIPE_STATE=""
for _ in $(seq 1 50); do
  PIPE_STATE="$("${CLI_BIN}" list --json \
    | python3 -c "import json,sys; tasks=json.load(sys.stdin); print(next(t['state'] for t in tasks if t['id']=='${PIPE_TID}'))")"
  [[ "${PIPE_STATE}" == "COMPLETED" || "${PIPE_STATE}" == "FAILED" ]] && break
  sleep 0.1
done
[[ "${PIPE_STATE}" == "COMPLETED" || "${PIPE_STATE}" == "FAILED" ]] \
  || fail "pipeline did not reach terminal state; saw '${PIPE_STATE}'"
ok "pipeline reached terminal state (${PIPE_STATE})"

echo
echo "all smoke checks passed"
