// Trigger configuration editor for the visual workflow canvas.
import type { WorkflowTrigger } from '../lib/workflowYaml'

interface TriggerEditorProps {
  triggers: WorkflowTrigger[]
  onChange: (triggers: WorkflowTrigger[]) => void
}

const TRIGGER_TYPES: Array<{ type: WorkflowTrigger['type']; label: string; helper: string }> = [
  { type: 'manual', label: 'Manual', helper: 'Run only when started by a user or API call.' },
  { type: 'cron', label: 'Cron schedule', helper: 'Run on a schedule (cron expression).' },
  { type: 'webhook', label: 'Webhook', helper: 'Trigger via HTTP POST to /webhooks/{name}.' },
  { type: 'event', label: 'Event', helper: 'Trigger on an r8r event (workflow/external).' },
]

function newTrigger(type: WorkflowTrigger['type']): WorkflowTrigger {
  switch (type) {
    case 'cron':
      return { type, schedule: '0 8 * * *', timezone: 'UTC' }
    case 'webhook':
      return { type, method: 'POST', path: '' }
    case 'event':
      return { type, event: '' }
    default:
      return { type }
  }
}

export default function TriggerEditor({ triggers, onChange }: TriggerEditorProps) {
  const update = (index: number, patch: Partial<WorkflowTrigger>) => {
    onChange(triggers.map((t, i) => (i === index ? { ...t, ...patch } : t)))
  }

  const remove = (index: number) => {
    onChange(triggers.filter((_, i) => i !== index))
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-700">
          Triggers
        </h3>
        <div className="flex gap-1">
          {TRIGGER_TYPES.map((tt) => (
            <button
              key={tt.type}
              onClick={() => onChange([...triggers, newTrigger(tt.type)])}
              className="rounded bg-slate-100 px-2 py-1 text-[11px] font-medium text-slate-600 hover:bg-emerald-50 hover:text-emerald-700"
              title={tt.helper}
            >
              + {tt.label}
            </button>
          ))}
        </div>
      </div>

      {triggers.length === 0 && (
        <p className="text-xs text-slate-500">
          No triggers — the workflow will only be actionable from the API/CLI. Add one above.
        </p>
      )}

      {triggers.map((trigger, index) => (
        <div key={index} className="rounded-md border border-slate-200 bg-slate-50 p-3">
          <div className="flex items-center justify-between gap-2">
            <span className="text-xs font-semibold text-slate-700">
              {TRIGGER_TYPES.find((t) => t.type === trigger.type)?.label ?? trigger.type}
            </span>
            <button
              onClick={() => remove(index)}
              className="text-xs text-rose-600 hover:text-rose-800"
            >
              Hapus
            </button>
          </div>

          {trigger.type === 'cron' && (
            <div className="mt-2 grid grid-cols-2 gap-2">
              <label className="text-xs text-slate-600">
                Schedule (cron)
                <input
                  type="text"
                  value={trigger.schedule ?? ''}
                  onChange={(e) => update(index, { schedule: e.target.value })}
                  placeholder="0 8 * * *"
                  className="mt-1 w-full rounded-md border border-slate-300 px-2 py-1 font-mono text-xs focus:border-emerald-500 focus:outline-none"
                />
              </label>
              <label className="text-xs text-slate-600">
                Timezone
                <input
                  type="text"
                  value={trigger.timezone ?? ''}
                  onChange={(e) => update(index, { timezone: e.target.value })}
                  placeholder="UTC"
                  className="mt-1 w-full rounded-md border border-slate-300 px-2 py-1 text-xs focus:border-emerald-500 focus:outline-none"
                />
              </label>
            </div>
          )}

          {trigger.type === 'webhook' && (
            <div className="mt-2 grid grid-cols-2 gap-2">
              <label className="text-xs text-slate-600">
                Path
                <input
                  type="text"
                  value={trigger.path ?? ''}
                  onChange={(e) => update(index, { path: e.target.value })}
                  placeholder="/webhooks/my-flow"
                  className="mt-1 w-full rounded-md border border-slate-300 px-2 py-1 font-mono text-xs focus:border-emerald-500 focus:outline-none"
                />
              </label>
              <label className="text-xs text-slate-600">
                Method
                <select
                  value={trigger.method ?? 'POST'}
                  onChange={(e) => update(index, { method: e.target.value })}
                  className="mt-1 w-full rounded-md border border-slate-300 px-2 py-1 text-xs focus:border-emerald-500 focus:outline-none"
                >
                  {['POST', 'GET', 'PUT', 'PATCH', 'DELETE'].map((m) => (
                    <option key={m}>{m}</option>
                  ))}
                </select>
              </label>
            </div>
          )}

          {trigger.type === 'event' && (
            <div className="mt-2 grid grid-cols-2 gap-2">
              <label className="text-xs text-slate-600">
                Event name
                <input
                  type="text"
                  value={trigger.event ?? ''}
                  onChange={(e) => update(index, { event: e.target.value })}
                  placeholder="workflow.completed"
                  className="mt-1 w-full rounded-md border border-slate-300 px-2 py-1 font-mono text-xs focus:border-emerald-500 focus:outline-none"
                />
              </label>
              <label className="text-xs text-slate-600">
                Filter (optional)
                <input
                  type="text"
                  value={trigger.filter ?? ''}
                  onChange={(e) => update(index, { filter: e.target.value })}
                  placeholder='status == "ok"'
                  className="mt-1 w-full rounded-md border border-slate-300 px-2 py-1 font-mono text-xs focus:border-emerald-500 focus:outline-none"
                />
              </label>
            </div>
          )}
        </div>
      ))}
    </div>
  )
}
