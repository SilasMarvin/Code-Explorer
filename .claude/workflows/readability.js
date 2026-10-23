export const meta = {
  name: 'readability',
  description: 'Screenshot codeviz scenarios, quiz agents on what each shows from the image alone, and score their answers',
  whenToUse: 'After changing how the graph or the journey draws, to measure whether readers get more of it right. args: {run, baseline?, only?, readers?, conditions?}',
  phases: [
    { title: 'Render', detail: 'screenshot each scenario bare and write its quiz' },
    { title: 'Quiz', detail: 'each reader model on each scenario and condition' },
    { title: 'Grade', detail: 'score the answers against what each view means to show' },
  ],
}

// args: run names the output in target/readability (default "latest"); baseline names
// an earlier run to compare with (default: the most recent other run); only limits the
// scenarios; readers are the reader models (default sonnet and haiku: one reader of a
// model answers the same way every time, so a second model is what says how robust a
// reading is, and haiku is weak enough that small text and faint marks cost it points);
// conditions are "cold" (the image alone) and "primed" (with a paragraph on how to
// read it).
const opts = args ?? {}
const run = opts.run ?? 'latest'
const models = opts.readers ?? ['sonnet', 'haiku']
const conditions = opts.conditions ?? ['cold']
const only = (opts.only ?? []).join(',')
const script = 'python3 eval/readability/readability.py'

const RENDERED = {
  type: 'object',
  properties: {
    dir: { type: 'string' },
    scenarios: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          name: { type: 'string' },
          prompts: { type: 'object', additionalProperties: { type: 'string' } },
        },
        required: ['name', 'prompts'],
      },
    },
    error: { type: 'string' },
  },
  required: ['dir', 'scenarios'],
}

const ANSWERS = {
  type: 'object',
  properties: {
    answers: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          id: { type: 'string' },
          answer: { type: 'array', items: { type: 'string' } },
          cannot_tell: { type: 'boolean' },
          confidence: { type: 'number', minimum: 0, maximum: 1 },
        },
        required: ['id', 'answer', 'cannot_tell', 'confidence'],
      },
    },
  },
  required: ['answers'],
}

phase('Render')
const command = `${script} render --run ${run} --conditions ${conditions.join(',')}${only ? ` --only ${only}` : ''}`
const rendered = await agent(
  `From the repository root, run this command (it can take a few minutes; give it a 600000 ms timeout):\n\n${command}\n\n` +
    'Its last line of stdout is a JSON object: return that object exactly. If the command fails, ' +
    'return {"dir": "", "scenarios": [], "error": "<the error output>"}.',
  { phase: 'Render', schema: RENDERED, effort: 'low' },
)
if (!rendered || rendered.error || !rendered.scenarios.length) {
  return { run, error: rendered?.error ?? 'render failed' }
}
log(`${rendered.scenarios.length} scenarios rendered into ${rendered.dir}`)

phase('Quiz')
const readers = rendered.scenarios.flatMap(s =>
  conditions.flatMap(condition => models.map(model => ({ name: s.name, condition, model, prompt: s.prompts[condition] }))),
)
const answered = await parallel(
  readers.map(r => () =>
    agent(
      `Read the instructions in ${r.prompt} and follow them exactly. Read no file besides that one and ` +
        'the screenshot it names, and run no commands. Then write your answer object, as JSON, to ' +
        `${rendered.dir}/answers/${r.name}.${r.condition}.${r.model}.json with the Write tool, and return the same object.`,
      { phase: 'Quiz', label: `${r.name} ${r.condition} ${r.model}`, schema: ANSWERS, model: r.model },
    ),
  ),
)
const missing = readers.filter((_, i) => !answered[i]).map(r => `${r.name}.${r.condition}.${r.model}`)
if (missing.length) log(`no answers from ${missing.join(', ')}; they are left out of the score`)

phase('Grade')
const graded = await agent(
  `From the repository root, run:\n\n${script} grade --run ${run}${opts.baseline ? ` --baseline ${opts.baseline}` : ''}\n\n` +
    'Return its full stdout exactly, with nothing added.',
  { phase: 'Grade', effort: 'low' },
)
return { run, dir: rendered.dir, missing, report: graded }
