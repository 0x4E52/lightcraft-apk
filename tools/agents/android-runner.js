// Claude Code workflow: run task cards from docs/android-port.md one after another.
//
// Per card: fix forward a red GitHub run (CI or Android) first, implement the card with the model it names, run the
// fast gate (CLAUDE.md, docs/android-port.md → "Gate per task"), an adversarial review, up to two fix rounds, push.
// The chain stops at the first card that can't be finished green, so the pushed branch always ends at a clean task
// boundary. Uncommitted work exists only while a card runs.
//
// Run it from Claude Code in the repository (multi-agent orchestration must be asked for explicitly):
//   "Run the workflow tools/agents/android-runner.js with args
//    {"branch": "<your branch>", "tasks": [{"id": "APK-29", "model": "sonnet"}, …], "attribution": "<trailer lines>"}"
// `tasks` follow docs/android-port.md → "Order of the remaining work"; `model` is the card's Model line.
// `attribution` is the commit-trailer text to end each commit message with (empty for none).
export const meta = {
  name: 'android-runner',
  description: 'Run docs/android-port.md task cards in order: fix red CI, implement, fast gate, review, fix, push',
  phases: [
    { title: 'CI check', detail: 'fix forward a red CI or Android run on the branch' },
    { title: 'Implement', detail: 'one agent per card, model as the card says' },
    { title: 'Review', detail: 'adversarial review of the commit' },
    { title: 'Fix', detail: 'address the findings, fast gate again' },
    { title: 'Push', detail: 'push and record the runs' },
  ],
}

const BRANCH = args.branch
const REPO = args.repo || '0x4E52/lightcraft-apk'
const TRAILER = args.attribution ? `Commit messages end with a blank line and exactly:\n${args.attribution}` : ''
const COMMON = `You work in the LightCraft repository (cwd) on branch ${BRANCH}. The plan is docs/android-port.md; follow its "Ground rules for the port", the rules under "Executing in this container" that apply where you run (the "Gate per task": fmt, clippy and tests of the touched crates incl. --target aarch64-linux-android for crates the APK links, plus the xtask checks the change touches, no local full \`cargo xtask ci\`; the disk precondition, cleaning only while \`pgrep -x cargo\` prints nothing) and the "Decisions for the maintainer" defaults. Phase 3 cards follow docs/android-touch-ui.md. Commands that may run over 10 minutes go to the background with a log file. Never tick a device or CI line you didn't see yourself. Never put a model identifier in commits, code or docs. ${TRAILER}`

const RESULT = {
  type: 'object',
  properties: {
    status: { type: 'string', enum: ['done', 'blocked'] },
    commit: { type: 'string' },
    gate: { type: 'string', enum: ['green', 'red', 'not-run'] },
    summary: { type: 'string' },
    blocker: { type: 'string' },
  },
  required: ['status', 'commit', 'gate', 'summary', 'blocker'],
}
const REVIEW = {
  type: 'object',
  properties: {
    ok: { type: 'boolean' },
    issues: { type: 'array', items: { type: 'object', properties: {
      severity: { type: 'string', enum: ['must-fix', 'should-fix', 'nit'] },
      where: { type: 'string' }, problem: { type: 'string' }, fix: { type: 'string' },
    }, required: ['severity', 'where', 'problem', 'fix'] } },
  },
  required: ['ok', 'issues'],
}
const CICHECK = {
  type: 'object',
  properties: {
    state: { type: 'string', enum: ['green', 'fixed', 'running', 'none', 'blocked'] },
    commit: { type: 'string' },
    summary: { type: 'string' },
  },
  required: ['state', 'commit', 'summary'],
}

const done = []
for (const t of args.tasks) {
  const card = `task card ${t.id} in docs/android-port.md`
  phase('CI check')
  const ci = await agent(`${COMMON}\n\nBefore ${t.id}: (1) disk: if \`df -h /\` shows less than 10 GB free and \`pgrep -x cargo\` prints nothing, remove target/debug/examples and target/debug/incremental, and if still under 10 GB remove target/debug entirely. (2) CI: for each workflow named "CI" and "Android", find its most recent COMPLETED run on branch ${BRANCH} (\`gh api "repos/${REPO}/actions/runs?branch=${BRANCH}&status=completed&per_page=10"\`; cancelled runs don't count). If every latest one succeeded (or none exists), report green/none and change nothing. If one failed: find the failing step (\`gh api repos/${REPO}/actions/runs/<id>/jobs\`; the job log via \`gh api repos/${REPO}/actions/jobs/<job_id>/logs\`, or the check-run annotations \`gh api repos/${REPO}/check-runs/<job_id>/annotations\` when the log can't be fetched), reproduce it with the narrowest local command, fix the root cause (never skip or disable a test, never weaken a check that is right), run the fast gate for what you touched, make ONE commit "ci: <what was fixed>" and push it (\`git push -u origin ${BRANCH}\`, retry 4x on network errors). Report fixed and the commit, or blocked with why.`,
    { label: `ci-check:${t.id}`, phase: 'CI check', model: 'sonnet', schema: CICHECK })
  if (!ci || ci.state === 'blocked') {
    log(`${t.id}: stopped, a red run could not be fixed (${ci ? ci.summary : 'agent failed'})`)
    return { done, stopped_at: t.id, ci }
  }
  if (ci.state === 'fixed') log(`fixed forward before ${t.id}: ${ci.commit}`)

  phase('Implement')
  const impl = await agent(`${COMMON}\n\nImplement ${card}, and only that card. Read the card, the results of its "Depends on" cards in git log, and the code it names first. Do exactly its Steps, write its Tests, run its Verify commands with the fast gate in place of a local full ci. Then make ONE commit "${t.id}: <short title>" whose body says what and why. Do not push. If the card can't be done as written (a fact in it is wrong, an environment need is missing), do as much of it as is verifiable here, keep the gate green, and report "blocked" with the reason; never commit with a red gate.`,
    { label: `impl:${t.id}`, phase: 'Implement', model: t.model, schema: RESULT })
  if (!impl || impl.status !== 'done' || impl.gate !== 'green') {
    log(`${t.id}: stopped (${impl ? `${impl.status}, gate ${impl.gate}: ${impl.blocker}` : 'agent failed'})`)
    return { done, stopped_at: t.id, result: impl }
  }
  let commit = impl.commit
  let review = null
  for (let round = 1; round <= 2; round++) {
    phase('Review')
    review = await agent(`${COMMON}\n\nAdversarially review commit ${commit} (\`git show ${commit}\`), which claims to implement ${card}. Look for: card steps or tests that are missing, a test that would pass without the change, a panic path (unwrap/expect/indexing on input, unsafe outside the allowed crates), cfg(target_os = "android") outside the leaves the card allows, a god module or god script, a layering break, a doc or parity row not updated, a device or CI check claimed as done, a model id in the commit. Run the quick parts of the fast gate for the touched crates. ok=true only with no must-fix and no should-fix. Don't edit files.`,
      { label: `review:${t.id}:r${round}`, phase: 'Review', model: 'opus', schema: REVIEW })
    if (!review || review.ok) break
    phase('Fix')
    const fix = await agent(`${COMMON}\n\nA reviewer found these issues in commit ${commit} for ${card}. Fix every must-fix and should-fix (nits only when trivial; reject an issue only when it is wrong, with the evidence). Run the fast gate for what you touched, then fold the fixes into the same commit with \`git commit --amend\` (it is not pushed yet), keeping its message. Report the new hash.\n\nIssues:\n${JSON.stringify(review.issues, null, 1)}`,
      { label: `fix:${t.id}:r${round}`, phase: 'Fix', model: t.model, schema: RESULT })
    if (!fix || fix.status !== 'done' || fix.gate !== 'green') {
      log(`${t.id}: fix round ${round} failed (${fix ? fix.blocker : 'agent failed'})`)
      return { done, stopped_at: t.id, result: fix, review }
    }
    commit = fix.commit
  }
  if (review && !review.ok) log(`${t.id}: pushed with open review issues after two fix rounds`)
  phase('Push')
  const push = await agent(`In the repository (cwd), check that HEAD is ${commit} on branch ${BRANCH} and the working tree is clean, then \`git push -u origin ${BRANCH}\`, retrying up to 4 times on network errors (2 s, 4 s, 8 s, 16 s). Wait about 30 s and list the runs for the pushed commit: \`gh api "repos/${REPO}/actions/runs?head_sha=<full sha>" --jq '.workflow_runs[] | "\\(.name) \\(.status) \\(.html_url)"'\`. Return the pushed hash and the run URLs, or the error.`,
    { label: `push:${t.id}`, phase: 'Push', model: 'sonnet', effort: 'low' })
  done.push({ id: t.id, commit, summary: impl.summary, open_issues: review && !review.ok ? review.issues : [], push })
  log(`${t.id}: pushed (${commit})`)
}
return { done, stopped_at: null }
