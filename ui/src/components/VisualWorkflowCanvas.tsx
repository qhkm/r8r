import { useCallback, useMemo, useState } from 'react'
import ReactFlow, {
  type Connection,
  type Edge,
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

type SaveState = 'idle' | 'saving' | 'saved' | 'error'

interface CanvasNodeData {
  label: string
  nodeType: string
  color: string
  config: Record<string, unknown>
}

interface WorkflowNode {
  id: string
  type: string
  config: Record<string, unknown>
  depends_on?: string[]
}

interface WorkflowDefinition {
  name: string
  description: string
  version: number
  triggers: Array<Record<string, unknown>>
  nodes: WorkflowNode[]
}

const NODE_LIBRARY = [
  { nodeType: 'http', label: 'HTTP Request', color: '#2563eb' },
  { nodeType: 'transform', label: 'Transform', color: '#7c3aed' },
  { nodeType: 'filter', label: 'Filter', color: '#059669' },
  { nodeType: 'agent', label: 'AI Agent', color: '#d97706' },
  { nodeType: 'if', label: 'Condition', color: '#dc2626' },
  { nodeType: 'switch', label: 'Switch', color: '#ea580c' },
  { nodeType: 'merge', label: 'Merge', color: '#0891b2' },
  { nodeType: 'split', label: 'Split', color: '#65a30d' },
  { nodeType: 'wait', label: 'Wait', color: '#475569' },
  { nodeType: 'set', label: 'Set Variables', color: '#4338ca' },
  { nodeType: 'approval', label: 'Approval Gate', color: '#0f766e' },
  { nodeType: 'debug', label: 'Debug', color: '#6d28d9' },
]

function sanitizeWorkflowName(raw: string): string {
  const fallback = 'visual_workflow'
  const normalized = raw
    .trim()
    .toLowerCase()
    .replace(/\s+/g, '_')
    .replace(/[^a-z0-9_-]/g, '')
    .replace(/_+/g, '_')

  return normalized.length > 0 ? normalized : fallback
}

function buildDefinition(
  workflowName: string,
  workflowDescription: string,
  nodes: Node<CanvasNodeData>[],
  edges: Edge[]
): WorkflowDefinition {
  const workflowNodes: WorkflowNode[] = nodes.map((node) => {
    const dependencies = edges.filter((edge) => edge.target === node.id).map((edge) => edge.source)

    return {
      id: node.id,
      type: node.data.nodeType,
      config: node.data.config ?? {},
      ...(dependencies.length > 0 ? { depends_on: dependencies } : {}),
    }
  })

  return {
    name: sanitizeWorkflowName(workflowName),
    description: workflowDescription,
    version: 1,
    triggers: [{ type: 'manual' }],
    nodes: workflowNodes,
  }
}

export default function VisualWorkflowCanvas() {
  const [nodes, setNodes, onNodesChange] = useNodesState<CanvasNodeData>([])
  const [edges, setEdges, onEdgesChange] = useEdgesState([])
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null)
  const [workflowName, setWorkflowName] = useState('visual_workflow')
  const [workflowDescription, setWorkflowDescription] = useState('')
  const [saveState, setSaveState] = useState<SaveState>('idle')
  const [feedback, setFeedback] = useState<string | null>(null)

  const selectedNode = useMemo(
    () => nodes.find((node) => node.id === selectedNodeId) ?? null,
    [nodes, selectedNodeId]
  )

  const definition = useMemo(
    () => buildDefinition(workflowName, workflowDescription, nodes, edges),
    [workflowName, workflowDescription, nodes, edges]
  )

  const definitionText = useMemo(() => JSON.stringify(definition, null, 2), [definition])

  const onConnect = useCallback(
    (connection: Connection) => {
      setEdges((existing) => addEdge({ ...connection, animated: true }, existing))
    },
    [setEdges]
  )

  const addNode = (nodeType: string, label: string, color: string) => {
    const id = `${nodeType}_${Date.now().toString(36)}_${Math.floor(Math.random() * 10000).toString(36)}`

    const newNode: Node<CanvasNodeData> = {
      id,
      type: 'default',
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
  }

  const updateSelectedNodeConfig = (rawJson: string) => {
    if (!selectedNode) {
      return
    }

    try {
      const parsed = JSON.parse(rawJson) as Record<string, unknown>
      setNodes((existing) =>
        existing.map((node) =>
          node.id === selectedNode.id
            ? {
                ...node,
                data: {
                  ...node.data,
                  config: parsed,
                },
              }
            : node
        )
      )
    } catch {
      // Keep invalid text untouched in the textarea until user fixes JSON.
    }
  }

  const deleteSelectedNode = () => {
    if (!selectedNode) {
      return
    }

    const targetId = selectedNode.id
    setNodes((existing) => existing.filter((node) => node.id !== targetId))
    setEdges((existing) => existing.filter((edge) => edge.source !== targetId && edge.target !== targetId))
    setSelectedNodeId(null)
  }

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

  return (
    <div className="flex h-[calc(100vh-96px)] min-h-[680px] flex-col">
      <div className="border-b border-slate-200 bg-white px-5 py-4">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex flex-wrap items-center gap-3">
            <h2 className="text-lg font-semibold text-slate-900">Advanced Visual Editor</h2>
            <input
              type="text"
              value={workflowName}
              onChange={(event) => setWorkflowName(event.target.value)}
              placeholder="workflow_name"
              className="rounded-md border border-slate-300 px-3 py-1.5 text-sm focus:border-emerald-500 focus:outline-none focus:ring-2 focus:ring-emerald-100"
            />
            <input
              type="text"
              value={workflowDescription}
              onChange={(event) => setWorkflowDescription(event.target.value)}
              placeholder="Workflow description"
              className="w-72 rounded-md border border-slate-300 px-3 py-1.5 text-sm focus:border-emerald-500 focus:outline-none focus:ring-2 focus:ring-emerald-100"
            />
          </div>

          <div className="flex items-center gap-2">
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
      </div>

      <div className="flex flex-1 overflow-hidden">
        <aside className="w-64 overflow-y-auto border-r border-slate-200 bg-white p-4">
          <h3 className="mb-3 text-sm font-semibold uppercase tracking-wide text-slate-700">Node palette</h3>
          <div className="space-y-2">
            {NODE_LIBRARY.map((node) => (
              <button
                key={node.nodeType}
                onClick={() => addNode(node.nodeType, node.label, node.color)}
                className="flex w-full items-center gap-3 rounded-md px-3 py-2 text-left text-sm text-slate-700 transition hover:bg-slate-100"
              >
                <span className="h-3 w-3 rounded-full" style={{ backgroundColor: node.color }} />
                {node.label}
              </button>
            ))}
          </div>
        </aside>

        <div className="flex-1 bg-slate-50">
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            onNodeClick={(_, node) => setSelectedNodeId(node.id)}
            fitView
          >
            <Background />
            <Controls />
            <MiniMap />
            <Panel position="top-left" className="rounded-md border border-slate-200 bg-white px-3 py-2 text-xs text-slate-600">
              Connect nodes to define execution order
            </Panel>
          </ReactFlow>
        </div>

        <aside className="w-96 overflow-y-auto border-l border-slate-200 bg-white p-4">
          <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-700">Node details</h3>

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
                <label className="mb-1 block text-xs font-medium text-slate-600">Node config (JSON)</label>
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

          <div className="mt-6">
            <h4 className="text-sm font-semibold text-slate-800">Definition preview</h4>
            <pre className="mt-2 max-h-64 overflow-auto rounded-md bg-slate-950 p-3 text-xs text-slate-100">{definitionText}</pre>
          </div>
        </aside>
      </div>
    </div>
  )
}
