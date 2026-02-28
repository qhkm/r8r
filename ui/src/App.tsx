import { useState } from 'react'
import WorkflowEditor from './components/WorkflowEditor'
import './index.css'

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
  duration_ms?: number
}

function Dashboard() {
  const [activeTab, setActiveTab] = useState<'workflows' | 'executions'>('workflows')
  const [workflows, setWorkflows] = useState<Workflow[]>([])
  const [executions, setExecutions] = useState<Execution[]>([])
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const fetchData = async (tab: 'workflows' | 'executions') => {
    setLoading(true)
    try {
      if (tab === 'workflows') {
        const res = await fetch('/api/workflows')
        const data = await res.json()
        setWorkflows(data.workflows || [])
      } else {
        // Note: This endpoint doesn't exist yet, would need to be added
        setExecutions([])
      }
      setError(null)
    } catch (err) {
      setError('Failed to fetch data from API')
    } finally {
      setLoading(false)
    }
  }

  const formatDate = (dateStr: string) => {
    return new Date(dateStr).toLocaleString()
  }

  const getStatusColor = (status: string) => {
    switch (status.toLowerCase()) {
      case 'completed':
        return 'bg-green-100 text-green-800'
      case 'failed':
        return 'bg-red-100 text-red-800'
      case 'running':
        return 'bg-blue-100 text-blue-800'
      default:
        return 'bg-gray-100 text-gray-800'
    }
  }

  return (
    <div className="min-h-screen bg-slate-50">
      {/* Header */}
      <header className="bg-white border-b border-slate-200">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <div className="flex justify-between items-center h-16">
            <div className="flex items-center gap-3">
              <div className="w-8 h-8 bg-indigo-600 rounded-lg flex items-center justify-center">
                <span className="text-white font-bold text-lg">r</span>
              </div>
              <h1 className="text-xl font-semibold text-slate-900">r8r Dashboard</h1>
            </div>
            <div className="flex items-center gap-4">
              <a 
                href="/editor" 
                className="px-4 py-2 bg-indigo-600 text-white rounded-md text-sm font-medium hover:bg-indigo-700 transition-colors"
              >
                + New Workflow
              </a>
              <a 
                href="/api/monitor" 
                target="_blank"
                className="text-sm text-indigo-600 hover:text-indigo-700 font-medium"
              >
                Live Monitor →
              </a>
            </div>
          </div>
        </div>
      </header>

      {/* Navigation */}
      <nav className="bg-white border-b border-slate-200">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8">
          <div className="flex gap-8">
            <button
              onClick={() => { setActiveTab('workflows'); fetchData('workflows') }}
              className={`py-4 px-1 border-b-2 font-medium text-sm transition-colors ${
                activeTab === 'workflows'
                  ? 'border-indigo-600 text-indigo-600'
                  : 'border-transparent text-slate-500 hover:text-slate-700'
              }`}
            >
              Workflows
            </button>
            <button
              onClick={() => { setActiveTab('executions'); fetchData('executions') }}
              className={`py-4 px-1 border-b-2 font-medium text-sm transition-colors ${
                activeTab === 'executions'
                  ? 'border-indigo-600 text-indigo-600'
                  : 'border-transparent text-slate-500 hover:text-slate-700'
              }`}
            >
              Executions
            </button>
          </div>
        </div>
      </nav>

      {/* Main Content */}
      <main className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-8">
        {error && (
          <div className="mb-4 p-4 bg-red-50 border border-red-200 rounded-lg text-red-700">
            {error}
          </div>
        )}

        {loading ? (
          <div className="flex items-center justify-center h-64">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-indigo-600"></div>
          </div>
        ) : activeTab === 'workflows' ? (
          <div className="bg-white rounded-lg shadow-sm border border-slate-200">
            <div className="px-6 py-4 border-b border-slate-200 flex justify-between items-center">
              <h2 className="text-lg font-semibold text-slate-900">Workflows</h2>
              <span className="text-sm text-slate-500">{workflows.length} total</span>
            </div>
            <div className="divide-y divide-slate-200">
              {workflows.length === 0 ? (
                <div className="px-6 py-8 text-center text-slate-500">
                  No workflows found. Create one with the CLI:{' '}
                  <code className="bg-slate-100 px-2 py-1 rounded text-sm">r8r workflows create</code>
                  {' '}or use the <a href="/editor" className="text-indigo-600 hover:underline">Visual Editor</a>
                </div>
              ) : (
                workflows.map((wf) => (
                  <div key={wf.id} className="px-6 py-4 hover:bg-slate-50 transition-colors">
                    <div className="flex items-center justify-between">
                      <div>
                        <h3 className="text-sm font-medium text-slate-900">{wf.name}</h3>
                        <p className="text-sm text-slate-500 mt-1">
                          {wf.node_count} nodes • {wf.trigger_count} triggers
                        </p>
                      </div>
                      <div className="flex items-center gap-4">
                        <span
                          className={`px-2.5 py-0.5 rounded-full text-xs font-medium ${
                            wf.enabled
                              ? 'bg-green-100 text-green-800'
                              : 'bg-gray-100 text-gray-800'
                          }`}
                        >
                          {wf.enabled ? 'Enabled' : 'Disabled'}
                        </span>
                        <span className="text-sm text-slate-400">
                          Updated {formatDate(wf.updated_at)}
                        </span>
                      </div>
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>
        ) : (
          <div className="bg-white rounded-lg shadow-sm border border-slate-200">
            <div className="px-6 py-4 border-b border-slate-200 flex justify-between items-center">
              <h2 className="text-lg font-semibold text-slate-900">Recent Executions</h2>
              <span className="text-sm text-slate-500">{executions.length} total</span>
            </div>
            <div className="divide-y divide-slate-200">
              {executions.length === 0 ? (
                <div className="px-6 py-8 text-center text-slate-500">
                  No executions yet. Run a workflow with:{' '}
                  <code className="bg-slate-100 px-2 py-1 rounded text-sm">r8r workflows run</code>
                </div>
              ) : (
                executions.map((exec) => (
                  <div key={exec.id} className="px-6 py-4 hover:bg-slate-50 transition-colors">
                    <div className="flex items-center justify-between">
                      <div>
                        <div className="flex items-center gap-3">
                          <h3 className="text-sm font-medium text-slate-900">{exec.workflow_name}</h3>
                          <span
                            className={`px-2.5 py-0.5 rounded-full text-xs font-medium ${getStatusColor(
                              exec.status
                            )}`}
                          >
                            {exec.status}
                          </span>
                        </div>
                        <p className="text-sm text-slate-500 mt-1">
                          {exec.trigger_type} • Started {formatDate(exec.started_at)}
                        </p>
                      </div>
                      <div className="text-right">
                        {exec.duration_ms && (
                          <span className="text-sm text-slate-500">
                            {exec.duration_ms}ms
                          </span>
                        )}
                        <p className="text-xs text-slate-400 font-mono mt-1">{exec.id.slice(0, 8)}...</p>
                      </div>
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>
        )}
      </main>

      {/* Footer */}
      <footer className="bg-white border-t border-slate-200 mt-auto">
        <div className="max-w-7xl mx-auto px-4 sm:px-6 lg:px-8 py-4">
          <p className="text-sm text-slate-500 text-center">
            r8r Workflow Engine • <a href="/api/health" className="text-indigo-600 hover:text-indigo-700">Health Check</a>
          </p>
        </div>
      </footer>
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
