export const meta = {
  name: 'issue-fix',
  description: 'Analyse a GitHub issue, design a mitigation, implement, verify and review the fix',
  phases: [
    { title: 'Analyse', detail: 'three lenses verify the issue against the real code' },
    { title: 'Design', detail: 'competing mitigation plans, then a synthesis' },
    { title: 'Implement', detail: 'one agent makes the change and runs the checks' },
    { title: 'Review', detail: 'standards and spec reviewers on the diff' },
    { title: 'Remediate', detail: 'apply blocking review findings' },
  ],
}

const N = args.number
const REPO = [
  'The repo is RAIL, a Tauri app: Rust backend in src-tauri/, React+TS frontend in src/.',
  'Read CLAUDE.md and docs/CONVENTIONS.md first and obey them exactly.',
  'Checks: `npm run lint`, `npm run typecheck`, `npm test`, and inside src-tauri: `cargo clippy --all-targets --all-features -- -D warnings` and `cargo test`.',
  'Issue text: run `gh issue view ' + N + '` (add --comments for the thread).',
].join('\n')

const ANALYSIS = {
  type: 'object',
  properties: {
    lens: { type: 'string' },
    verified_claims: { type: 'array', items: { type: 'string' } },
    stale_or_wrong_claims: { type: 'array', items: { type: 'string' } },
    key_files: { type: 'array', items: { type: 'string' } },
    risks: { type: 'array', items: { type: 'string' } },
    notes: { type: 'string' },
  },
  required: ['lens', 'verified_claims', 'stale_or_wrong_claims', 'key_files', 'risks', 'notes'],
}

const PLAN = {
  type: 'object',
  properties: {
    summary: { type: 'string' },
    steps: { type: 'array', items: { type: 'string' } },
    files_touched: { type: 'array', items: { type: 'string' } },
    tests: { type: 'array', items: { type: 'string' } },
    out_of_scope: { type: 'array', items: { type: 'string' } },
    risk: { type: 'string' },
  },
  required: ['summary', 'steps', 'files_touched', 'tests', 'out_of_scope', 'risk'],
}

const IMPL = {
  type: 'object',
  properties: {
    done: { type: 'boolean' },
    what_changed: { type: 'string' },
    acceptance_criteria: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          criterion: { type: 'string' },
          met: { type: 'boolean' },
          note: { type: 'string' },
        },
        required: ['criterion', 'met', 'note'],
      },
    },
    checks: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          command: { type: 'string' },
          passed: { type: 'boolean' },
          output: { type: 'string' },
        },
        required: ['command', 'passed', 'output'],
      },
    },
    left_undone: { type: 'string' },
  },
  required: ['done', 'what_changed', 'acceptance_criteria', 'checks', 'left_undone'],
}

const REVIEW = {
  type: 'object',
  properties: {
    verdict: { type: 'string', enum: ['approve', 'changes-requested'] },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          file: { type: 'string' },
          severity: { type: 'string', enum: ['blocking', 'minor'] },
          summary: { type: 'string' },
        },
        required: ['file', 'severity', 'summary'],
      },
    },
    notes: { type: 'string' },
  },
  required: ['verdict', 'findings', 'notes'],
}

phase('Analyse')
const LENSES = [
  ['claims', 'Verify every factual claim in the issue against the code as it exists TODAY. The issue may be stale: file/line references may have moved and some of the work may already be done. Report exactly which claims hold and which do not, with file:line evidence. Do not change any files.'],
  ['context', 'Map the code the issue touches: the modules, call sites, existing tests, and any adjacent code that would break if this changed. Read widely. Do not change any files.'],
  ['risk', 'Find what could go wrong if this issue is fixed: behavioural regressions, hidden couplings, overlap with other open issues (`gh issue list`), and anything that argues for a narrower scope. Do not change any files.'],
]
const analyses = (await parallel(LENSES.map(([lens, task]) => () =>
  agent(REPO + '\n\nYou are the "' + lens + '" analyst for issue #' + N + '.\n' + task, { label: 'analyse:' + lens, phase: 'Analyse', schema: ANALYSIS })
))).filter(Boolean)

const analysisText = JSON.stringify(analyses, null, 2)

phase('Design')
const ANGLES = [
  ['minimal', 'Design the SMALLEST change that satisfies the acceptance criteria. Prefer deleting code. Reject speculative abstraction.'],
  ['structural', 'Design the change the issue actually argues for, taking the architectural refactor seriously. Justify every new interface by at least two callers.'],
]
const plans = (await parallel(ANGLES.map(([angle, task]) => () =>
  agent(REPO + '\n\nYou are the "' + angle + '" designer for issue #' + N + '. ' + task + '\n\nAnalyst findings (treat as evidence; verify anything load-bearing):\n' + analysisText + '\n\nProduce a concrete, file-level implementation plan. Do not change any files.', { label: 'design:' + angle, phase: 'Design', schema: PLAN })
))).filter(Boolean)

const chosen = await agent(REPO + '\n\nYou are the design judge for issue #' + N + '. Two plans were proposed:\n' + JSON.stringify(plans, null, 2) + '\n\nAnalyst findings:\n' + analysisText + '\n\nPick the plan that best satisfies the issue acceptance criteria at acceptable risk, grafting the better ideas from the other. Anything the acceptance criteria do not require goes in out_of_scope. If the current code already satisfies the issue, say so in summary and return an empty steps list. Do not change any files.', { label: 'design:judge', phase: 'Design', schema: PLAN })

if (!chosen || !chosen.steps || chosen.steps.length === 0) {
  log('issue #' + N + ': judge produced no steps, nothing to implement')
  return { number: N, implemented: false, plan: chosen, analyses }
}

phase('Implement')
const IMPL_RULES = [
  'Rules:',
  '- Match the surrounding code style. No new dependencies without checking the CLAUDE.md approved list.',
  '- Add or update tests where the plan says so.',
  '- Run every check listed above and make them ALL pass before you finish. Report the real output; never claim a check passed if it did not.',
  '- Do NOT commit, do NOT touch git history, do NOT push.',
  '- Report faithfully what you left undone.',
].join('\n')
const impl = await agent(REPO + '\n\nImplement the fix for issue #' + N + ' in the working tree. Follow this agreed plan; do not widen scope:\n' + JSON.stringify(chosen, null, 2) + '\n\n' + IMPL_RULES, { label: 'implement:#' + N, phase: 'Implement', schema: IMPL })

phase('Review')
const REVIEWERS = [
  ['standards', 'Review the uncommitted diff (`git diff` and `git status`) against CLAUDE.md and docs/CONVENTIONS.md: no unwrap outside tests, no `any`, doc comments on public Rust items, file size limits, no business logic in the frontend, no silently invented mock data. Also hunt for correctness bugs introduced by the diff.'],
  ['spec', 'Review the uncommitted diff against the issue acceptance criteria one by one. Mark any criterion the diff claims but does not actually meet as a blocking finding. Re-run the checks yourself rather than trusting the implementer report.'],
]
const reviews = (await parallel(REVIEWERS.map(([lens, task]) => () =>
  agent(REPO + '\n\nYou are the "' + lens + '" reviewer for issue #' + N + '. ' + task + '\n\nImplementer report:\n' + JSON.stringify(impl, null, 2) + '\n\nDo not change any files; report only.', { label: 'review:' + lens, phase: 'Review', schema: REVIEW })
))).filter(Boolean)

const blocking = reviews.flatMap(r => r.findings.filter(f => f.severity === 'blocking'))

let remediation = null
if (blocking.length > 0) {
  phase('Remediate')
  remediation = await agent(REPO + '\n\nFix these blocking review findings on issue #' + N + ' in the working tree:\n' + JSON.stringify(blocking, null, 2) + '\n\nFull reviews for context:\n' + JSON.stringify(reviews, null, 2) + '\n\nIf a finding is wrong, say so instead of changing code. Re-run all the checks and make them pass. Do NOT commit.', { label: 'remediate:#' + N, phase: 'Remediate', schema: IMPL })
}

return { number: N, implemented: true, plan: chosen, impl, reviews, blocking, remediation }
