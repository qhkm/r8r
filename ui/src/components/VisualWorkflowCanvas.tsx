import { useCallback, useMemo, useState } from 'react'
import ReactFlow, {
  type Connection,
  type Node,
  addEdge,
  Background,
  Controls,
  MiniMap,
  Panel,
  useEdgesState,
  useNodesState,
} from 'reactflow'
import 'reactflow/dist/style.css'
import TriggerEditor from './TriggerEditor'
import {
  buildDefinitionFromCanvas,
  parseDefinitionToCanvas,
  sanitizeWorkflowName,
  NODE_COLORS,
  NODE_LIBRARY,
  type CanvasNodeData,
  type WorkflowTrigger,
} from '../lib/workflowYaml'

type SaveState = 'idle' | 'saving' | 'saved' | 'error'

interface WorkflowSummary {
  id: string
  name: string
  enabled: boolean
  node_count: number
  trigger_count: number
  updated_at: string
}

const NODE_GROUPS = Array.from(new Set(NODE_LIBRARY.map((n) => n.group)))

export default function VisualWorkflowCanvas() {
  const [nodes, setNodes, onNodesChange] = useNodesState<CanvasNodeData>([])
  const [edges, setEdges, onEdgesChange] = useEdgesState([])
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)
  const [workflowName, setWorkflowName] = useState('visual_workflow')
  const [workflowDescription, setWorkflowDescription] = useState('')
  const [triggers, setTriggers] = useState<WorkflowTrigger[]>([{ type: 'manual' }])
  const [saveState, setSaveState] = useState<SaveState>('idle')
  const [feedback, setFeedback] = useState<string | null>(null)

  // Workflow loader state
  const [workflows, setWorkflows] = useState<WorkflowSummary[] | null>(null)
  const [loaderOpen, setLoaderOpen] = useState(false)
  const [loadingWorkflow, setLoadingWorkflow] = useState<string | null>(null)
  const [dirty, setDirty] = useState(false)

  const selectedNode = useMemo(
    () => nodes.find((node) => node.id === selectedNodeId) ?? null,
    [nodes, selectedNodeId]
  )

  const definition = useMemo(
    () => buildDefinitionFromCanvas(workflowName, workflowDescription, nodes, edges, triggers),
    [workflowName, workflowDescription, nodes, edges, triggers]
  )

  const definitionText = useMemo(() => JSON.stringify(definition, null, 2), [definition])

  const markDirty = useCallback(() => setDirty(true), [])

  const onConnect = useCallback(
    (connection: Connection) => {
      setEdges((existing) => addEdge({ ...connection, animated: true }, existing))
      markDirty()
    },
    [setEdges, markDirty]
  )

  const addNode = (nodeType: string, label: string) => {
    const id = `${nodeType}_${Date.now().toString(36)}_${Math.floor(Math.random() * 10000).toString(36)}`
    const color = NODE_COLORS[nodeType] ?? '#64748b'

    const newNode: Node<CanvasNodeData> = {
      id,
      position: {
        x: 120 + Math.floor(Math.random() * 360),
        y: 100 + Math.floor(Math.random() * 240),
      },
      data: {
        label,
        nodeType,
        color,
        config: {},
      },
      style: {
        background: color,
        color: '#ffffff',
        border: 'none',
        borderRadius: '10px',
        padding: '10px 14px',
        fontWeight: 600,
      },
    }

    setNodes((existing) => [...existing, newNode])
    setSelectedNodeId(id)
    markDirty()
  }

  const updateSelectedNodeConfig = (rawJson: string) => {
    if (!selectedNode) return
    try {
      const parsed = JSON.parse(rawJson) as Record<string, unknown>
      setNodes((existing) =>
        existing.map((node) =>
          node.id === selectedNode.id
            ? { ...node, data: { ...node.data, config: parsed } }
            : node
        )
      )
      markDirty()
    } catch {
      // Keep invalid text untouched in the textarea until user fixes JSON.
    }
  }

  const deleteSelectedNode = () => {
    if (!selectedNode) return
    const targetId = selectedNode.id
    setNodes((existing) => existing.filter((node) => node.id !== targetId))
    setEdges((existing) => existing.filter((edge) => edge.source !== targetId && edge.target !== targetId))
    setSelectedNodeId(null)
    markDirty()
  }

  // ---- Workflow loading ----

  const openLoader = async () => {
    setFeedback(null)
    try {
      const response = await fetch('/api/workflows')
      if (!response.ok) {
        const body = (await response.json().catch(() => ({}))) as { error?: { message?: string } }
        throw new Error(body.error?.message ?? 'Failed to list workflows')
      }
      const body = (await response.json()) as { workflows?: WorkflowSummary[] }
      setWorkflows(body.workflows ?? [])
      setLoaderOpen(true)
    } catch (error) {
      setFeedback(error instanceof Error ? error.message : 'Failed to list workflows')
    }
  }

  const loadWorkflow = async (name: string) => {
    setLoadingWorkflow(name)
    setFeedback(null)
    try {
      const response = await fetch(`/api/workflows/${encodeURIComponent(name)}`)
      if (!response.ok) {
        const body = (await response.json().catch(() => ({}))) as { error?: { message?: string } }
        throw new Error(body.error?.message ?? 'Failed to load workflow')
      }
      const body = (await response.json()) as { name: string; definition: string }
      const loaded = parseDefinitionToCanvas(body.definition)
      setWorkflowName(loaded.workflowName)
      setWorkflowDescription(loaded.description)
      setNodes(loaded.nodes)
      setEdges(loaded.edges)
      setTriggers(loaded.triggers)
      setSelectedNodeId(null)
      setDirty(false)
      setFeedback(`Loaded workflow "${loaded.name}".`)
    } catch (error) {
      setFeedback(error instanceof Error ? error.message : 'Failed to load workflow')
    } finally {
      setLoadingWorkflow(null)
    }
  }

  // ---- Save ----

  const saveWorkflow = async () => {
    setSaveState('saving')
    setFeedback(null)
    try {
      const safeName = sanitizeWorkflowName(workflowName)
      const response = await fetch('/api/workflows', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: safeName,
          definition: definitionText,
          enabled: true,
        }),
      })
      if (!response.ok) {
        const body = (await response.json().catch(() => ({}))) as { error?: { message?: string } }
        throw new Error(body.error?.message ?? 'Save failed')
      }
      setSaveState('saved')
      setDirty(false)
      setFeedback(`Workflow ${safeName} was saved.`)
    } catch (error) {
      setSaveState('error')
      setFeedback(error instanceof Error ? error.message : 'Save failed')
    }
  }

  const copyDefinition = async () => {
    try {
      await navigator.clipboard.writeText(definitionText)
      setFeedback('Definition copied to clipboard.')
    } catch {
      setFeedback('Clipboard access failed. Copy manually from the preview.')
    }
  }

  const groups = NODE_GROUPS

  return (
    <div className="flex h-[calc(100vh-96px)] min-h-[680px] flex-col">
      <div className="border-b border-slate-200 bg-white px-5 py-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex flex-wrap items-center gap-3">
            <h2 className="text-lg font-semibold text-slate-900">Advanced Visual Editor</h2>
            <input
              type="text"
              value={workflowName}
              onChange={(event) => {
                setWorkflowName(event.target.value)
                markDirty()
              }}
              placeholder="workflow_name"
              className="rounded-md border border-slate-300 px-3 py-1.5 text-sm focus:border-emerald-500 focus:outline-none focus:ring-2 focus:ring-emerald-100"
            />
            <input
              type="text"
              value={workflowDescription}
              onChange={(event) => {
                setWorkflowDescription(event.target.value)
                markDirty()
              }}
              placeholder="Workflow description"
              className="w-72 rounded-md border border-slate-300 px-3 py-1.5 text-sm focus:border-emerald-500 focus:outline-none focus:ring-2 focus:ring-emerald-100"
            />
            {dirty && (
              <span className="rounded-full bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-800">
                Unsaved changes
              </span>
            )}
          </div>

          <div className="flex items-center gap-2">
            <button
              onClick={() => void openLoader()}
              className="rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100"
            >
              Open workflow
            </button>
            <button
              onClick={() => void copyDefinition()}
              className="rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100"
            >
              Copy definition
            </button>
            <button
              onClick={() => void saveWorkflow()}
              disabled={saveState === 'saving'}
              className="rounded-md bg-emerald-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-emerald-700 disabled:cursor-not-allowed disabled:opacity-65"
            >
              {saveState === 'saving' ? 'Saving...' : 'Save workflow'}
            </button>
          </div>
        </div>

        {feedback && (
          <p
            className={`mt-3 rounded-md px-3 py-2 text-sm ${
              saveState === 'error'
                ? 'border border-rose-200 bg-rose-50 text-rose-700'
                : 'border border-emerald-200 bg-emerald-50 text-emerald-700'
            }`}
          >
            {feedback}
          </p>
        )}

        {loaderOpen && (
          <div className="mt-3 rounded-md border border-slate-200 bg-slate-50 p-3">
            <div className="mb-2 flex items-center justify-between">
              <h3 className="text-sm font-semibold text-slate-700">Existing workflows</h3>
              <button
                onClick={() => setLoaderOpen(false)}
                className="text-xs text-slate-500 hover:text-slate-800"
              >
                Tutup
              </button>
            </div>
            {workflows === null ? (
              <p className="text-xs text-slate-500">Loading…</p>
            ) : workflows.length === 0 ? (
              <p className="text-xs text-slate-500">No workflows found. Click a node to start building.</p>
            ) : (
              <div className="max-h-56 overflow-y-auto">
                {workflows.map((wf) => (
                  <button
                    key={wf.id}
                    onClick={() => void loadWorkflow(wf.name)}
                    disabled={loadingWorkflow === wf.name}
                    className="flex w-full items-center justify-between gap-3 rounded-md px-3 py-2 text-left text-sm hover:bg-white disabled:opacity-60"
                  >
                    <span className="font-medium text-slate-800">{wf.name}</span>
                    <span className="text-xs text-slate-500">
                      {wf.node_count} nodes · {wf.trigger_count} triggers ·{' '}
                      {wf.enabled ? 'enabled' : 'disabled'}
                    </span>
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      <div className="flex flex-1 overflow-hidden">
        <aside className="w-64 overflow-y-auto border-r border-slate-200 bg-white p-4">
          <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate-700">
            Node palette
          </h3>
          {groups.map((group) => (
            <div key={group} className="mb-4">
              <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-slate-400">
                {group}
              </h4>
              <div className="space-y-2">
                {NODE_LIBRARY.filter((node) => node.group === group).map((node) => (
                  <button
                    key={node.nodeType}
                    onClick={() => addNode(node.nodeType, node.label)}
                    className="flex w-full items-center gap-3 rounded-md px-3 py-2 text-left text-sm text-slate-700 transition hover:bg-slate-100"
                  >
                    <span
                      className="h-3 w-3 rounded-full"
                      style={{ backgroundColor: NODE_COLORS[node.nodeType] ?? '#64748b' }}
                    />
                    {node.label}
                  </button>
                ))}
              </div>
            </div>
          ))}
        </aside>

        <div className="flex-1 bg-slate-50">
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={(changes) => {
              onNodesChange(changes)
              markDirty()
            }}
            onEdgesChange={(changes) => {
              onEdgesChange(changes)
              markDirty()
            }}
            onConnect={onConnect}
            onNodeClick={(_, node) => setSelectedNodeId(node.id)}
            onPaneClick={() => setSelectedNodeId(null)}
            fitView
          >
            <Background />
            <Controls />
            <MiniMap />
            <Panel
              position="top-left"
              className="rounded-md border border-slate-200 bg-white px-3 py-2 text-xs text-slate-600"
            >
              Connect nodes to define execution order
            </Panel>
          </ReactFlow>
        </div>

        <aside className="w-96 overflow-y-auto border-l border-slate-200 bg-white p-4">
          <TriggerEditor triggers={triggers} onChange={(next) => { setTriggers(next); markDirty() }} />

          <div className="mt-6">
            <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-700">
              Node details
            </h3>

            {selectedNode ? (
              <div className="mt-3 space-y-4">
                <div>
                  <label className="mb-1 block text-xs font-medium text-slate-600">Node ID</label>
                  <input
                    type="text"
                    value={selectedNode.id}
                    readOnly
                    className="w-full rounded-md border border-slate-200 bg-slate-100 px-3 py-2 text-sm"
                  />
                </div>

                <div>
                  <label className="mb-1 block text-xs font-medium text-slate-600">Node type</label>
                  <input
                    type="text"
                    value={selectedNode.data.nodeType}
                    readOnly
                    className="w-full rounded-md border border-slate-200 bg-slate-100 px-3 py-2 text-sm"
                  />
                </div>

                <div>
                  <label className="mb-1 block text-xs font-medium text-slate-600">
                    Node config (JSON)
                  </label>
                  <textarea
                    rows={9}
                    value={JSON.stringify(selectedNode.data.config ?? {}, null, 2)}
                    onChange={(event) => updateSelectedNodeConfig(event.target.value)}
                    className="w-full rounded-md border border-slate-300 px-3 py-2 font-mono text-xs focus:border-emerald-500 focus:outline-none focus:ring-2 focus:ring-emerald-100"
                  />
                </div>

                <button
                  onClick={deleteSelectedNode}
                  className="w-full rounded-md bg-rose-50 px-3 py-2 text-sm font-medium text-rose-700 hover:bg-rose-100"
                >
                  Delete node
                </button>
              </div>
            ) : (
              <p className="mt-3 text-sm text-slate-500">Select a node to configure it.</p>
            )}
          </div>

          <div className="mt-6">
            <h4 className="text-sm font-semibold text-slate-800">Definition preview</h4>
            <pre className="mt-2 max-h-64 overflow-auto rounded-md bg-slate-950 p-3 text-xs text-slate-100">
              {definitionText}
            </pre>
          </div>
        </aside>
      </div>
    </div>
  )
}
