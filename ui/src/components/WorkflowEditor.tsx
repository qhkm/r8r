import { useEffect, useMemo, useState } from 'react'
import VisualWorkflowCanvas from './VisualWorkflowCanvas'

type TriggerMode = 'manual' | 'hourly' | 'daily' | 'weekly'
type AlertMode = 'none' | 'dashboard' | 'email' | 'slack'
type EditorMode = 'guided' | 'visual'

interface StudioTemplate {
  id: string
  title: string
  description: string
  suggestedName: string
  defaultGoal: string
  defaultApproval: boolean
}

interface StudioFormState {
  workflowName: string
  summary: string
  sourceSystem: string
  triggerMode: TriggerMode
  alertMode: AlertMode
  requiresApproval: boolean
  templateId: string
}

const TEMPLATES: StudioTemplate[] = [
  {
    id: 'ops_daily_check',
    title: 'Daily operations check',
    description: 'Collect daily metrics, summarize findings, and notify your team.',
    suggestedName: 'daily_ops_check',
    defaultGoal: 'Create a daily operations summary and highlight anomalies.',
    defaultApproval: false,
  },
  {
    id: 'lead_followup',
    title: 'Lead follow-up',
    description: 'Capture inbound leads and assign the next action quickly.',
    suggestedName: 'lead_followup',
    defaultGoal: 'Capture new leads and route them to the right owner.',
    defaultApproval: false,
  },
  {
    id: 'approval_gate',
    title: 'Approval gate',
    description: 'Pause sensitive actions until someone explicitly approves.',
    suggestedName: 'approval_gate',
    defaultGoal: 'Require a human review step before final actions run.',
    defaultApproval: true,
  },
]

interface WorkflowNode {
  id: string
  type: string
  depends_on?: string[]
  config: Record<string, unknown>
}

interface WorkflowDefinition {
  name: string
  description: string
  version: number
  triggers: Array<Record<string, unknown>>
  nodes: WorkflowNode[]
}

const triggerCopy: Record<TriggerMode, { label: string; schedule: string | null; helper: string }> = {
  manual: { label: 'Manual', schedule: null, helper: 'Run only when started by a user or API call.' },
  hourly: { label: 'Hourly', schedule: '0 * * * *', helper: 'Runs at minute 0 of every hour.' },
  daily: { label: 'Daily', schedule: '0 8 * * *', helper: 'Runs daily at 08:00 server time.' },
  weekly: { label: 'Weekly', schedule: '0 8 * * 1', helper: 'Runs every Monday at 08:00 server time.' },
}

const alertCopy: Record<AlertMode, string> = {
  none: 'No notification step will be added.',
  dashboard: 'A dashboard note is produced in the run output.',
  email: 'A placeholder email action is prepared for your team.',
  slack: 'A placeholder Slack action is prepared for your team.',
}

function sanitizeWorkflowName(raw: string): string {
  const fallback = 'guided_workflow'
  const sanitized = raw
    .trim()
    .toLowerCase()
    .replace(/\s+/g, '_')
    .replace(/[^a-z0-9_-]/g, '')
    .replace(/_+/g, '_')

  return sanitized.length > 0 ? sanitized : fallback
}

function buildWorkflowDefinition(form: StudioFormState): WorkflowDefinition {
  const safeName = sanitizeWorkflowName(form.workflowName)
  const selectedTemplate = TEMPLATES.find((template) => template.id === form.templateId) ?? TEMPLATES[0]
  const trigger = triggerCopy[form.triggerMode]

  const nodes: WorkflowNode[] = [
    {
      id: 'capture_context',
      type: 'set',
      config: {
        fields: [
          { name: 'summary', value: form.summary },
          { name: 'source_system', value: form.sourceSystem },
          { name: 'template', value: selectedTemplate.id },
          { name: 'created_from', value: 'guided_studio' },
        ],
      },
    },
    {
      id: 'prepare_action',
      type: 'debug',
      depends_on: ['capture_context'],
      config: {
        message: `Preparing workflow plan for ${selectedTemplate.title}`,
        label: 'guided.plan',
      },
    },
  ]

  let finalDependency = 'prepare_action'

  if (form.requiresApproval) {
    nodes.push({
      id: 'request_approval',
      type: 'approval',
      depends_on: ['prepare_action'],
      config: {
        title: `Approve execution for ${safeName}`,
        description: form.summary,
        timeout_seconds: 3600,
        default_action: 'reject',
      },
    })
    finalDependency = 'request_approval'
  }

  if (form.alertMode !== 'none') {
    nodes.push({
      id: 'notify_team',
      type: 'debug',
      depends_on: [finalDependency],
      config: {
        label: `notify.${form.alertMode}`,
        message: `Notification prepared for ${form.alertMode}`,
      },
    })
    finalDependency = 'notify_team'
  }

  nodes.push({
    id: 'finish',
    type: 'debug',
    depends_on: [finalDependency],
    config: {
      label: 'guided.finish',
      message: 'Guided workflow completed.',
    },
  })

  return {
    name: safeName,
    description: form.summary,
    version: 1,
    triggers: trigger.schedule
      ? [{ type: 'cron', schedule: trigger.schedule }]
      : [{ type: 'manual' }],
    nodes,
  }
}

function describePlan(form: StudioFormState): string[] {
  const steps = [
    `Capture context from ${form.sourceSystem || 'your selected source'}.`,
    'Prepare an execution plan with readable logs.',
  ]

  if (form.requiresApproval) {
    steps.push('Pause for approval before final actions.')
  }

  if (form.alertMode !== 'none') {
    steps.push(`Notify the team via ${form.alertMode}.`)
  }

  steps.push('Finish and store run output for auditing.')
  return steps
}

export default function WorkflowEditor() {
  const query = useMemo(() => new URLSearchParams(window.location.search), [])
  const initialTemplate = query.get('template')
  const startingTemplate = TEMPLATES.find((template) => template.id === initialTemplate)?.id ?? TEMPLATES[0].id
  const initialMode: EditorMode = query.get('mode') === 'visual' ? 'visual' : 'guided'

  const [mode, setMode] = useState<EditorMode>(initialMode)
  const [form, setForm] = useState<StudioFormState>({
    workflowName: TEMPLATES.find((template) => template.id === startingTemplate)?.suggestedName ?? 'guided_workflow',
    summary: TEMPLATES.find((template) => template.id === startingTemplate)?.defaultGoal ?? '',
    sourceSystem: 'Operations dashboard',
    triggerMode: 'manual',
    alertMode: 'dashboard',
    requiresApproval: TEMPLATES.find((template) => template.id === startingTemplate)?.defaultApproval ?? false,
    templateId: startingTemplate,
  })

  const [saveState, setSaveState] = useState<'idle' | 'saving' | 'saved' | 'error'>('idle')
  const [feedback, setFeedback] = useState<string | null>(null)

  const selectedTemplate = useMemo(
    () => TEMPLATES.find((template) => template.id === form.templateId) ?? TEMPLATES[0],
    [form.templateId]
  )

  useEffect(() => {
    setForm((previous) => ({
      ...previous,
      workflowName: previous.workflowName ? previous.workflowName : selectedTemplate.suggestedName,
      summary: previous.summary ? previous.summary : selectedTemplate.defaultGoal,
      requiresApproval: selectedTemplate.defaultApproval,
    }))
  }, [selectedTemplate.defaultApproval, selectedTemplate.defaultGoal, selectedTemplate.suggestedName])

  const safeWorkflowName = useMemo(() => sanitizeWorkflowName(form.workflowName), [form.workflowName])
  const workflowDefinition = useMemo(() => buildWorkflowDefinition(form), [form])
  const definitionText = useMemo(() => JSON.stringify(workflowDefinition, null, 2), [workflowDefinition])
  const planSteps = useMemo(() => describePlan(form), [form])

  const setField = <K extends keyof StudioFormState>(key: K, value: StudioFormState[K]) => {
    setForm((previous) => ({ ...previous, [key]: value }))
  }

  const setEditorMode = (nextMode: EditorMode) => {
    setMode(nextMode)
    const params = new URLSearchParams(window.location.search)

    if (nextMode === 'visual') {
      params.set('mode', 'visual')
    } else {
      params.delete('mode')
    }

    const suffix = params.toString()
    window.history.replaceState({}, '', suffix ? `/editor?${suffix}` : '/editor')
  }

  const saveWorkflow = async () => {
    setSaveState('saving')
    setFeedback(null)

    try {
      const response = await fetch('/api/workflows', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: safeWorkflowName,
          definition: definitionText,
          enabled: true,
        }),
      })

      if (!response.ok) {
        const body = (await response.json().catch(() => ({}))) as { error?: { message?: string } }
        throw new Error(body.error?.message ?? 'Workflow save failed')
      }

      setSaveState('saved')
      setFeedback(`Workflow ${safeWorkflowName} was created and enabled.`)
    } catch (error) {
      setSaveState('error')
      setFeedback(error instanceof Error ? error.message : 'Workflow save failed')
    }
  }

  const copyDefinition = async () => {
    try {
      await navigator.clipboard.writeText(definitionText)
      setFeedback('Definition copied to clipboard.')
    } catch {
      setFeedback('Clipboard access failed. Copy manually from preview.')
    }
  }

  const headerTitle = mode === 'guided' ? 'Build without YAML' : 'Advanced visual workflow editor'
  const headerDescription =
    mode === 'guided'
      ? 'Choose an outcome, fill plain-language details, and publish.'
      : 'Use drag-and-drop nodes for full control over workflow structure.'

  return (
    <div className="min-h-screen bg-[radial-gradient(circle_at_top_right,_#fef9e8,_#f8fafc_55%)]">
      <header className="border-b border-slate-200 bg-white/85 backdrop-blur">
        <div className="mx-auto flex w-full max-w-7xl flex-wrap items-center justify-between gap-4 px-4 py-4 sm:px-6 lg:px-8">
          <div>
            <p className="text-xs font-semibold uppercase tracking-[0.18em] text-emerald-700">workflow studio</p>
            <h1 className="text-2xl font-semibold text-slate-900">{headerTitle}</h1>
            <p className="text-sm text-slate-600">{headerDescription}</p>
          </div>

          <div className="flex items-center gap-2">
            <div className="rounded-lg border border-slate-300 bg-white p-1">
              <button
                onClick={() => setEditorMode('guided')}
                className={`rounded-md px-3 py-1.5 text-sm font-medium transition ${
                  mode === 'guided' ? 'bg-emerald-100 text-emerald-900' : 'text-slate-600 hover:bg-slate-100'
                }`}
              >
                Guided
              </button>
              <button
                onClick={() => setEditorMode('visual')}
                className={`rounded-md px-3 py-1.5 text-sm font-medium transition ${
                  mode === 'visual' ? 'bg-emerald-100 text-emerald-900' : 'text-slate-600 hover:bg-slate-100'
                }`}
              >
                Advanced Visual
              </button>
            </div>
            <a href="/" className="rounded-lg border border-slate-300 px-3 py-2 text-sm font-medium text-slate-700 hover:bg-slate-100">
              Back to dashboard
            </a>
          </div>
        </div>
      </header>

      {mode === 'visual' ? (
        <VisualWorkflowCanvas />
      ) : (
        <main className="mx-auto grid w-full max-w-7xl gap-6 px-4 py-8 sm:px-6 lg:grid-cols-[1.2fr_1fr] lg:px-8">
          <section className="space-y-6 rounded-2xl border border-slate-200 bg-white p-6 shadow-sm">
            <div>
              <h2 className="text-lg font-semibold text-slate-900">1. Pick a starter template</h2>
              <p className="text-sm text-slate-600">Templates provide safe defaults for non-technical operators.</p>
              <div className="mt-4 grid gap-3 md:grid-cols-3">
                {TEMPLATES.map((template) => {
                  const active = template.id === form.templateId
                  return (
                    <button
                      key={template.id}
                      onClick={() => {
                        setField('templateId', template.id)
                        setField('workflowName', template.suggestedName)
                        setField('summary', template.defaultGoal)
                        setField('requiresApproval', template.defaultApproval)
                      }}
                      className={`rounded-xl border px-4 py-3 text-left transition ${
                        active
                          ? 'border-emerald-400 bg-emerald-50'
                          : 'border-slate-200 bg-slate-50 hover:border-emerald-300 hover:bg-emerald-50'
                      }`}
                    >
                      <p className="text-sm font-semibold text-slate-900">{template.title}</p>
                      <p className="mt-1 text-xs text-slate-600">{template.description}</p>
                    </button>
                  )
                })}
              </div>
            </div>

            <div className="grid gap-4 md:grid-cols-2">
              <label className="text-sm font-medium text-slate-700">
                Workflow name
                <input
                  value={form.workflowName}
                  onChange={(event) => setField('workflowName', event.target.value)}
                  className="mt-1 w-full rounded-lg border border-slate-300 px-3 py-2 text-sm focus:border-emerald-400 focus:outline-none focus:ring-2 focus:ring-emerald-100"
                  placeholder="daily_ops_check"
                />
                <span className="mt-1 block text-xs text-slate-500">Saved as <code>{safeWorkflowName}</code></span>
              </label>

              <label className="text-sm font-medium text-slate-700">
                Source system
                <input
                  value={form.sourceSystem}
                  onChange={(event) => setField('sourceSystem', event.target.value)}
                  className="mt-1 w-full rounded-lg border border-slate-300 px-3 py-2 text-sm focus:border-emerald-400 focus:outline-none focus:ring-2 focus:ring-emerald-100"
                  placeholder="CRM, POS, ERP, ticketing"
                />
              </label>
            </div>

            <label className="block text-sm font-medium text-slate-700">
              2. Describe the business outcome
              <textarea
                rows={4}
                value={form.summary}
                onChange={(event) => setField('summary', event.target.value)}
                className="mt-1 w-full rounded-lg border border-slate-300 px-3 py-2 text-sm focus:border-emerald-400 focus:outline-none focus:ring-2 focus:ring-emerald-100"
                placeholder="Explain what this workflow should do in plain language"
              />
            </label>

            <div className="grid gap-4 md:grid-cols-2">
              <label className="text-sm font-medium text-slate-700">
                3. Trigger
                <select
                  value={form.triggerMode}
                  onChange={(event) => setField('triggerMode', event.target.value as TriggerMode)}
                  className="mt-1 w-full rounded-lg border border-slate-300 px-3 py-2 text-sm focus:border-emerald-400 focus:outline-none focus:ring-2 focus:ring-emerald-100"
                >
                  {(Object.keys(triggerCopy) as TriggerMode[]).map((triggerMode) => (
                    <option key={triggerMode} value={triggerMode}>
                      {triggerCopy[triggerMode].label}
                    </option>
                  ))}
                </select>
                <span className="mt-1 block text-xs text-slate-500">{triggerCopy[form.triggerMode].helper}</span>
              </label>

              <label className="text-sm font-medium text-slate-700">
                4. Notification style
                <select
                  value={form.alertMode}
                  onChange={(event) => setField('alertMode', event.target.value as AlertMode)}
                  className="mt-1 w-full rounded-lg border border-slate-300 px-3 py-2 text-sm focus:border-emerald-400 focus:outline-none focus:ring-2 focus:ring-emerald-100"
                >
                  <option value="none">No notifications</option>
                  <option value="dashboard">Dashboard note</option>
                  <option value="email">Email placeholder</option>
                  <option value="slack">Slack placeholder</option>
                </select>
                <span className="mt-1 block text-xs text-slate-500">{alertCopy[form.alertMode]}</span>
              </label>
            </div>

            <label className="flex items-start gap-3 rounded-lg border border-slate-200 bg-slate-50 p-3 text-sm text-slate-700">
              <input
                type="checkbox"
                checked={form.requiresApproval}
                onChange={(event) => setField('requiresApproval', event.target.checked)}
                className="mt-0.5 h-4 w-4 rounded border-slate-300 text-emerald-600 focus:ring-emerald-300"
              />
              <span>
                Require human approval before final actions.
                <span className="mt-1 block text-xs text-slate-500">Recommended for money movement, external sends, and policy-sensitive automation.</span>
              </span>
            </label>

            <div className="flex flex-wrap gap-2">
              <button
                onClick={() => void saveWorkflow()}
                disabled={saveState === 'saving'}
                className="rounded-lg bg-emerald-600 px-4 py-2 text-sm font-semibold text-white transition hover:bg-emerald-700 disabled:cursor-not-allowed disabled:opacity-65"
              >
                {saveState === 'saving' ? 'Publishing...' : 'Publish workflow'}
              </button>
              <button
                onClick={() => void copyDefinition()}
                className="rounded-lg border border-slate-300 px-4 py-2 text-sm font-medium text-slate-700 transition hover:bg-slate-100"
              >
                Copy definition
              </button>
              <button
                onClick={() => setEditorMode('visual')}
                className="rounded-lg border border-slate-300 px-4 py-2 text-sm font-medium text-slate-700 transition hover:bg-slate-100"
              >
                Open advanced visual editor
              </button>
            </div>

            {feedback && (
              <p
                className={`rounded-lg px-3 py-2 text-sm ${
                  saveState === 'error'
                    ? 'border border-rose-200 bg-rose-50 text-rose-700'
                    : 'border border-emerald-200 bg-emerald-50 text-emerald-700'
                }`}
              >
                {feedback}
              </p>
            )}
          </section>

          <section className="space-y-4 rounded-2xl border border-slate-200 bg-white p-6 shadow-sm">
            <div>
              <h2 className="text-lg font-semibold text-slate-900">Execution plan preview</h2>
              <p className="text-sm text-slate-600">This step list is what operators will see before run-time actions.</p>
              <ol className="mt-3 list-decimal space-y-1 pl-5 text-sm text-slate-700">
                {planSteps.map((step) => (
                  <li key={step}>{step}</li>
                ))}
              </ol>
            </div>

            <div className="rounded-xl border border-slate-200 bg-slate-950 p-4">
              <p className="mb-2 text-xs font-semibold uppercase tracking-wide text-slate-300">Definition (YAML-compatible JSON)</p>
              <pre className="max-h-[420px] overflow-auto text-xs text-slate-100">{definitionText}</pre>
            </div>

            <div className="rounded-xl border border-amber-200 bg-amber-50 p-3 text-xs text-amber-900">
              Template selected: <strong>{selectedTemplate.title}</strong>. You can adjust details at any time and republish.
            </div>
          </section>
        </main>
      )}
    </div>
  )
}
