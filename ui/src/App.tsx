import { useCallback, useEffect, useMemo, useState } from 'react'
import WorkflowEditor from './components/WorkflowEditor'
import './index.css'

type DashboardTab = 'overview' | 'workflows' | 'runs' | 'approvals' | 'audit'

interface Workflow {
  id: string
  name: string
  enabled: boolean
  node_count: number
  trigger_count: number
  updated_at: string
}

interface Execution {
  id: string
  workflow_name: string
  status: string
  trigger_type: string
  started_at: string
  finished_at?: string
  duration_ms?: number
  error?: string | null
}

interface Approval {
  id: string
  execution_id: string
  workflow_name: string
  title: string
  description?: string | null
  status: string
  created_at: string
  expires_at?: string | null
  decided_at?: string | null
}

interface AuditEntry {
  id: string
  timestamp: string
  event_type: string
  workflow_name?: string | null
  execution_id?: string | null
  detail: string
  actor: string
}

interface MetricCard {
  label: string
  value: string
  helper: string
}

const TAB_ORDER: DashboardTab[] = ['overview', 'workflows', 'runs', 'approvals', 'audit']

const TAB_META: Record<DashboardTab, { label: string; subtitle: string }> = {
  overview: {
    label: 'Overview',
    subtitle: 'Status snapshot and operator shortcuts',
  },
  workflows: {
    label: 'Workflows',
    subtitle: 'Manage automation definitions and run controls',
  },
  runs: {
    label: 'Runs',
    subtitle: 'Inspect recent execution outcomes',
  },
  approvals: {
    label: 'Approvals',
    subtitle: 'Handle human-in-the-loop decisions',
  },
  audit: {
    label: 'Audit',
    subtitle: 'Review platform actions and traceability',
  },
}

function Dashboard() {
  const [activeTab, setActiveTab] = useState<DashboardTab>('overview')
  const [workflows, setWorkflows] = useState<Workflow[]>([])
  const [executions, setExecutions] = useState<Execution[]>([])
  const [approvals, setApprovals] = useState<Approval[]>([])
  const [auditEntries, setAuditEntries] = useState<AuditEntry[]>([])
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState<string | null>(null)
  const [toast, setToast] = useState<string | null>(null)
  const [busyAction, setBusyAction] = useState<string | null>(null)

  const loadDashboard = useCallback(async () => {
    setLoading(true)
    try {
      const [workflowRes, executionRes, approvalRes, auditRes] = await Promise.all([
        fetch('/api/workflows'),
        fetch('/api/executions?limit=25'),
        fetch('/api/approvals?limit=25'),
        fetch('/api/audit?limit=25'),
      ])

      if (!workflowRes.ok || !executionRes.ok || !approvalRes.ok || !auditRes.ok) {
        throw new Error('One or more API requests failed')
      }

      const workflowData = (await workflowRes.json()) as { workflows?: Workflow[] }
      const executionData = (await executionRes.json()) as { executions?: Execution[] }
      const approvalData = (await approvalRes.json()) as { approvals?: Approval[] }
      const auditData = (await auditRes.json()) as { audit_entries?: AuditEntry[] }

      setWorkflows(workflowData.workflows ?? [])
      setExecutions(executionData.executions ?? [])
      setApprovals(approvalData.approvals ?? [])
      setAuditEntries(auditData.audit_entries ?? [])
      setError(null)
    } catch {
      setError('Could not load dashboard data. Check server status and refresh.')
    } finally {
      setLoading(false)
    }
  }, [])

  useEffect(() => {
    void loadDashboard()
  }, [loadDashboard])

  useEffect(() => {
    if (!toast) {
      return
    }

    const timeout = window.setTimeout(() => setToast(null), 3200)
    return () => window.clearTimeout(timeout)
  }, [toast])

  const pendingApprovals = useMemo(
    () => approvals.filter((approval) => approval.status === 'pending'),
    [approvals]
  )

  const metrics = useMemo<MetricCard[]>(() => {
    const activeWorkflows = workflows.filter((workflow) => workflow.enabled).length
    const runningExecutions = executions.filter((execution) => execution.status === 'running').length
    const failedExecutions = executions.filter((execution) => execution.status === 'failed').length

    return [
      {
        label: 'Active workflows',
        value: String(activeWorkflows),
        helper: `${workflows.length} total`,
      },
      {
        label: 'Runs in progress',
        value: String(runningExecutions),
        helper: `${executions.length} recent`,
      },
      {
        label: 'Pending approvals',
        value: String(pendingApprovals.length),
        helper: 'Awaiting decision',
      },
      {
        label: 'Recent failures',
        value: String(failedExecutions),
        helper: 'Last 25 runs',
      },
    ]
  }, [executions, pendingApprovals.length, workflows])

  const formatDate = (value?: string | null) => {
    if (!value) {
      return '—'
    }
    return new Date(value).toLocaleString()
  }

  const statusClass = (status: string) => {
    switch (status.toLowerCase()) {
      case 'completed':
      case 'approved':
      case 'enabled':
        return 'bg-emerald-100 text-emerald-800'
      case 'failed':
      case 'rejected':
        return 'bg-rose-100 text-rose-800'
      case 'running':
      case 'pending':
        return 'bg-amber-100 text-amber-900'
      default:
        return 'bg-slate-200 text-slate-700'
    }
  }

  const runWorkflow = async (workflowName: string) => {
    setBusyAction(`run:${workflowName}`)
    try {
      const response = await fetch(`/api/workflows/${encodeURIComponent(workflowName)}/execute`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ input: {}, wait: false }),
      })

      if (!response.ok) {
        throw new Error('run failed')
      }

      setToast(`Run started for ${workflowName}`)
      await loadDashboard()
    } catch {
      setToast(`Failed to start ${workflowName}`)
    } finally {
      setBusyAction(null)
    }
  }

  const decideApproval = async (approvalId: string, decision: 'approve' | 'reject') => {
    setBusyAction(`approval:${approvalId}:${decision}`)
    try {
      const response = await fetch(`/api/approvals/${encodeURIComponent(approvalId)}/decide`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          decision,
          comment: decision === 'approve' ? 'Approved from dashboard' : 'Rejected from dashboard',
          decided_by: 'dashboard',
        }),
      })

      if (!response.ok) {
        throw new Error('approval failed')
      }

      setToast(decision === 'approve' ? 'Approval accepted' : 'Approval rejected')
      await loadDashboard()
    } catch {
      setToast('Could not update approval status')
    } finally {
      setBusyAction(null)
    }
  }

  const renderOverview = () => (
    <section className="space-y-5">
      <div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
        {metrics.map((metric) => (
          <article key={metric.label} className="rounded-xl border border-slate-200 bg-white p-4 shadow-sm">
            <p className="text-xs uppercase tracking-wide text-slate-500">{metric.label}</p>
            <p className="mt-2 text-3xl font-semibold text-slate-900">{metric.value}</p>
            <p className="mt-1 text-xs text-slate-500">{metric.helper}</p>
          </article>
        ))}
      </div>

      <div className="grid gap-4 xl:grid-cols-[1.35fr_1fr]">
        <section className="rounded-xl border border-slate-200 bg-white p-5 shadow-sm">
          <div className="flex items-center justify-between gap-3">
            <h2 className="text-base font-semibold text-slate-900">Quick Start</h2>
            <a href="/editor" className="text-sm font-medium text-emerald-700 hover:text-emerald-800">
              Open guided studio
            </a>
          </div>
          <div className="mt-4 grid gap-3 md:grid-cols-3">
            {[
              {
                id: 'ops_daily_check',
                title: 'Daily Ops Check',
                detail: 'Daily business status and anomalies',
              },
              {
                id: 'lead_followup',
                title: 'Lead Follow-up',
                detail: 'Triage inbound leads quickly',
              },
              {
                id: 'approval_gate',
                title: 'Approval Gate',
                detail: 'Human sign-off before action',
              },
            ].map((template) => (
              <a
                key={template.id}
                href={`/editor?template=${template.id}`}
                className="rounded-lg border border-slate-200 bg-slate-50 p-3 transition hover:border-emerald-300 hover:bg-emerald-50"
              >
                <p className="text-sm font-semibold text-slate-900">{template.title}</p>
                <p className="mt-1 text-xs text-slate-600">{template.detail}</p>
              </a>
            ))}
          </div>
          <div className="mt-4 flex flex-wrap gap-2">
            <a
              href="/editor?mode=visual"
              className="rounded-md border border-slate-300 px-3 py-1.5 text-xs font-medium text-slate-700 hover:bg-slate-100"
            >
              Advanced visual editor
            </a>
            <a
              href="/editor"
              className="rounded-md bg-emerald-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-emerald-700"
            >
              New guided workflow
            </a>
          </div>
        </section>

        <section className="rounded-xl border border-slate-200 bg-white p-5 shadow-sm">
          <div className="flex items-center justify-between gap-3">
            <h2 className="text-base font-semibold text-slate-900">Pending Approvals</h2>
            <button
              onClick={() => setActiveTab('approvals')}
              className="text-sm font-medium text-emerald-700 hover:text-emerald-800"
            >
              Open queue
            </button>
          </div>
          {pendingApprovals.length === 0 ? (
            <p className="mt-3 text-sm text-slate-500">No pending approvals.</p>
          ) : (
            <div className="mt-3 space-y-2">
              {pendingApprovals.slice(0, 4).map((approval) => (
                <article key={approval.id} className="rounded-lg border border-amber-200 bg-amber-50 p-3">
                  <p className="text-sm font-semibold text-slate-900">{approval.title}</p>
                  <p className="text-xs text-slate-600">{approval.workflow_name}</p>
                  <div className="mt-2 flex gap-2">
                    <button
                      onClick={() => void decideApproval(approval.id, 'approve')}
                      disabled={busyAction === `approval:${approval.id}:approve`}
                      className="rounded-md bg-emerald-600 px-2.5 py-1 text-xs font-semibold text-white hover:bg-emerald-700 disabled:opacity-60"
                    >
                      Approve
                    </button>
                    <button
                      onClick={() => void decideApproval(approval.id, 'reject')}
                      disabled={busyAction === `approval:${approval.id}:reject`}
                      className="rounded-md bg-rose-600 px-2.5 py-1 text-xs font-semibold text-white hover:bg-rose-700 disabled:opacity-60"
                    >
                      Reject
                    </button>
                  </div>
                </article>
              ))}
            </div>
          )}
        </section>
      </div>

      <section className="rounded-xl border border-slate-200 bg-white shadow-sm">
        <div className="border-b border-slate-200 px-5 py-3">
          <h2 className="text-base font-semibold text-slate-900">Latest Runs</h2>
        </div>
        {executions.length === 0 ? (
          <p className="px-5 py-8 text-sm text-slate-500">No runs yet.</p>
        ) : (
          <div className="divide-y divide-slate-200">
            {executions.slice(0, 8).map((execution) => (
              <article key={execution.id} className="flex flex-wrap items-center justify-between gap-2 px-5 py-3">
                <div>
                  <p className="text-sm font-semibold text-slate-900">{execution.workflow_name}</p>
                  <p className="text-xs text-slate-500">{formatDate(execution.started_at)}</p>
                </div>
                <span className={`rounded-full px-2 py-0.5 text-xs font-medium ${statusClass(execution.status)}`}>
                  {execution.status}
                </span>
              </article>
            ))}
          </div>
        )}
      </section>
    </section>
  )

  const renderWorkflows = () => (
    <section className="rounded-xl border border-slate-200 bg-white shadow-sm">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-slate-200 px-5 py-4">
        <h2 className="text-base font-semibold text-slate-900">Workflow Catalog</h2>
        <div className="flex items-center gap-2">
          <a
            href="/editor"
            className="rounded-md bg-emerald-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-emerald-700"
          >
            Guided
          </a>
          <a
            href="/editor?mode=visual"
            className="rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100"
          >
            Visual
          </a>
        </div>
      </div>

      {workflows.length === 0 ? (
        <p className="px-5 py-8 text-sm text-slate-500">No workflows found.</p>
      ) : (
        <div className="divide-y divide-slate-200">
          {workflows.map((workflow) => (
            <article key={workflow.id} className="flex flex-wrap items-center justify-between gap-4 px-5 py-4">
              <div>
                <div className="flex items-center gap-2">
                  <p className="text-sm font-semibold text-slate-900">{workflow.name}</p>
                  <span
                    className={`rounded-full px-2 py-0.5 text-xs font-medium ${
                      workflow.enabled ? 'bg-emerald-100 text-emerald-800' : 'bg-slate-200 text-slate-700'
                    }`}
                  >
                    {workflow.enabled ? 'Enabled' : 'Disabled'}
                  </span>
                </div>
                <p className="mt-1 text-xs text-slate-500">
                  {workflow.node_count} nodes · {workflow.trigger_count} triggers · {formatDate(workflow.updated_at)}
                </p>
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => void runWorkflow(workflow.name)}
                  disabled={busyAction === `run:${workflow.name}`}
                  className="rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100 disabled:opacity-60"
                >
                  {busyAction === `run:${workflow.name}` ? 'Starting...' : 'Run'}
                </button>
                <a
                  href={`/editor?workflow=${encodeURIComponent(workflow.name)}`}
                  className="rounded-md bg-slate-900 px-3 py-1.5 text-sm font-medium text-white hover:bg-slate-800"
                >
                  Guided
                </a>
                <a
                  href={`/editor?mode=visual&workflow=${encodeURIComponent(workflow.name)}`}
                  className="rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100"
                >
                  Visual
                </a>
              </div>
            </article>
          ))}
        </div>
      )}
    </section>
  )

  const renderRuns = () => (
    <section className="rounded-xl border border-slate-200 bg-white shadow-sm">
      <div className="border-b border-slate-200 px-5 py-4">
        <h2 className="text-base font-semibold text-slate-900">Recent Runs</h2>
      </div>
      {executions.length === 0 ? (
        <p className="px-5 py-8 text-sm text-slate-500">No runs found.</p>
      ) : (
        <div className="divide-y divide-slate-200">
          {executions.map((execution) => (
            <article key={execution.id} className="grid gap-2 px-5 py-4 md:grid-cols-[1.4fr_auto_1fr] md:items-center">
              <div>
                <p className="text-sm font-semibold text-slate-900">{execution.workflow_name}</p>
                <p className="text-xs text-slate-500">{execution.id.slice(0, 8)} · {execution.trigger_type}</p>
              </div>
              <span className={`w-fit rounded-full px-2.5 py-1 text-xs font-semibold ${statusClass(execution.status)}`}>
                {execution.status}
              </span>
              <div className="text-xs text-slate-500">
                <p>{formatDate(execution.started_at)}</p>
                <p>{execution.duration_ms ?? 0} ms</p>
              </div>
            </article>
          ))}
        </div>
      )}
    </section>
  )

  const renderApprovals = () => (
    <section className="rounded-xl border border-slate-200 bg-white shadow-sm">
      <div className="border-b border-slate-200 px-5 py-4">
        <h2 className="text-base font-semibold text-slate-900">Approvals</h2>
      </div>
      {approvals.length === 0 ? (
        <p className="px-5 py-8 text-sm text-slate-500">No approvals found.</p>
      ) : (
        <div className="divide-y divide-slate-200">
          {approvals.map((approval) => (
            <article key={approval.id} className="grid gap-2 px-5 py-4 md:grid-cols-[1.5fr_auto_auto] md:items-center">
              <div>
                <p className="text-sm font-semibold text-slate-900">{approval.title}</p>
                <p className="text-xs text-slate-500">
                  {approval.workflow_name} · Created {formatDate(approval.created_at)}
                </p>
              </div>
              <span className={`w-fit rounded-full px-2.5 py-1 text-xs font-semibold ${statusClass(approval.status)}`}>
                {approval.status}
              </span>
              {approval.status === 'pending' ? (
                <div className="flex gap-2">
                  <button
                    onClick={() => void decideApproval(approval.id, 'approve')}
                    disabled={busyAction === `approval:${approval.id}:approve`}
                    className="rounded-md bg-emerald-600 px-2.5 py-1 text-xs font-semibold text-white hover:bg-emerald-700 disabled:opacity-60"
                  >
                    Approve
                  </button>
                  <button
                    onClick={() => void decideApproval(approval.id, 'reject')}
                    disabled={busyAction === `approval:${approval.id}:reject`}
                    className="rounded-md bg-rose-600 px-2.5 py-1 text-xs font-semibold text-white hover:bg-rose-700 disabled:opacity-60"
                  >
                    Reject
                  </button>
                </div>
              ) : (
                <p className="text-xs text-slate-500">Finalized {formatDate(approval.decided_at)}</p>
              )}
            </article>
          ))}
        </div>
      )}
    </section>
  )

  const renderAudit = () => (
    <section className="rounded-xl border border-slate-200 bg-white shadow-sm">
      <div className="border-b border-slate-200 px-5 py-4">
        <h2 className="text-base font-semibold text-slate-900">Audit Trail</h2>
      </div>
      {auditEntries.length === 0 ? (
        <p className="px-5 py-8 text-sm text-slate-500">No audit records found.</p>
      ) : (
        <div className="divide-y divide-slate-200">
          {auditEntries.map((entry) => (
            <article key={entry.id} className="grid gap-1 px-5 py-3 md:grid-cols-[1fr_auto] md:items-center">
              <div>
                <p className="text-sm font-semibold text-slate-900">{entry.detail}</p>
                <p className="text-xs text-slate-500">
                  {entry.event_type} · {entry.workflow_name ?? 'system'} · {entry.actor}
                </p>
              </div>
              <p className="text-xs text-slate-500">{formatDate(entry.timestamp)}</p>
            </article>
          ))}
        </div>
      )}
    </section>
  )

  const renderActiveTab = () => {
    switch (activeTab) {
      case 'overview':
        return renderOverview()
      case 'workflows':
        return renderWorkflows()
      case 'runs':
        return renderRuns()
      case 'approvals':
        return renderApprovals()
      case 'audit':
        return renderAudit()
      default:
        return renderOverview()
    }
  }

  const tabInfo = TAB_META[activeTab]

  return (
    <div className="min-h-screen bg-slate-100">
      <div className="flex min-h-screen">
        <aside className="hidden w-64 shrink-0 border-r border-slate-200 bg-white lg:flex lg:flex-col">
          <div className="border-b border-slate-200 px-5 py-5">
            <p className="text-xs font-semibold uppercase tracking-[0.18em] text-emerald-700">r8r</p>
            <h1 className="mt-1 text-xl font-semibold text-slate-900">Control Center</h1>
            <p className="mt-1 text-xs text-slate-500">Operator-first dashboard</p>
          </div>

          <nav className="flex-1 space-y-1 px-3 py-4">
            {TAB_ORDER.map((tab) => (
              <button
                key={tab}
                onClick={() => setActiveTab(tab)}
                className={`w-full rounded-lg px-3 py-2 text-left text-sm font-medium transition ${
                  activeTab === tab
                    ? 'bg-emerald-100 text-emerald-900'
                    : 'text-slate-600 hover:bg-slate-100 hover:text-slate-900'
                }`}
              >
                {TAB_META[tab].label}
              </button>
            ))}
          </nav>

          <div className="border-t border-slate-200 p-3">
            <a
              href="/editor"
              className="block rounded-md bg-emerald-600 px-3 py-2 text-center text-sm font-semibold text-white hover:bg-emerald-700"
            >
              New Guided Workflow
            </a>
            <a
              href="/editor?mode=visual"
              className="mt-2 block rounded-md border border-slate-300 px-3 py-2 text-center text-sm font-medium text-slate-700 hover:bg-slate-100"
            >
              Advanced Visual Editor
            </a>
          </div>
        </aside>

        <div className="flex min-w-0 flex-1 flex-col">
          <header className="border-b border-slate-200 bg-white">
            <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-4 sm:px-6">
              <div>
                <h2 className="text-xl font-semibold text-slate-900">{tabInfo.label}</h2>
                <p className="text-sm text-slate-600">{tabInfo.subtitle}</p>
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => void loadDashboard()}
                  className="rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100"
                >
                  Refresh
                </button>
                <a
                  href="/editor"
                  className="rounded-md bg-emerald-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-emerald-700"
                >
                  Guided
                </a>
                <a
                  href="/editor?mode=visual"
                  className="rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100"
                >
                  Visual
                </a>
              </div>
            </div>

            <div className="flex gap-1 overflow-x-auto border-t border-slate-100 px-3 py-2 lg:hidden">
              {TAB_ORDER.map((tab) => (
                <button
                  key={tab}
                  onClick={() => setActiveTab(tab)}
                  className={`shrink-0 rounded-md px-3 py-1.5 text-sm font-medium ${
                    activeTab === tab
                      ? 'bg-emerald-100 text-emerald-900'
                      : 'text-slate-600 hover:bg-slate-100'
                  }`}
                >
                  {TAB_META[tab].label}
                </button>
              ))}
            </div>
          </header>

          <main className="flex-1 px-4 py-5 sm:px-6">
            {error && (
              <div className="mb-4 rounded-lg border border-rose-200 bg-rose-50 px-4 py-3 text-sm text-rose-700">
                {error}
              </div>
            )}

            {loading ? (
              <div className="flex h-64 items-center justify-center">
                <div className="h-9 w-9 animate-spin rounded-full border-4 border-emerald-200 border-t-emerald-700" />
              </div>
            ) : (
              renderActiveTab()
            )}
          </main>
        </div>
      </div>

      {toast && (
        <div className="fixed bottom-4 right-4 rounded-lg border border-slate-300 bg-white px-4 py-2 text-sm text-slate-800 shadow-lg">
          {toast}
        </div>
      )}
    </div>
  )
}

function App() {
  const path = window.location.pathname

  if (path === '/editor') {
    return <WorkflowEditor />
  }

  return <Dashboard />
}

export default App
