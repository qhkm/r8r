import { useState, useCallback } from 'react'
import ReactFlow, {
  type Node,
  addEdge,
  Background,
  Controls,
  MiniMap,
  useNodesState,
  useEdgesState,
  type Connection,
  Panel,
} from 'reactflow'
import 'reactflow/dist/style.css'

interface WorkflowNode {
  id: string
  type: string
  config: Record<string, unknown>
  depends_on: string[]
  position?: { x: number; y: number }
}

interface Workflow {
  name: string
  description: string
  nodes: WorkflowNode[]
}

const nodeTypes = [
  { type: 'http', label: 'HTTP Request', color: '#3b82f6' },
  { type: 'transform', label: 'Transform', color: '#8b5cf6' },
  { type: 'filter', label: 'Filter', color: '#10b981' },
  { type: 'agent', label: 'AI Agent', color: '#f59e0b' },
  { type: 'if', label: 'Condition', color: '#ef4444' },
  { type: 'switch', label: 'Switch', color: '#f97316' },
  { type: 'merge', label: 'Merge', color: '#06b6d4' },
  { type: 'split', label: 'Split', color: '#84cc16' },
  { type: 'wait', label: 'Wait', color: '#64748b' },
  { type: 'set', label: 'Set Variables', color: '#6366f1' },
  { type: 'crypto', label: 'Crypto', color: '#ec4899' },
  { type: 'datetime', label: 'DateTime', color: '#14b8a6' },
  { type: 'debug', label: 'Debug', color: '#a855f7' },
]

export default function WorkflowEditor() {
  const [nodes, setNodes, onNodesChange] = useNodesState([])
  const [edges, setEdges, onEdgesChange] = useEdgesState([])
  const [selectedNode, setSelectedNode] = useState<Node | null>(null)
  const [workflowName, setWorkflowName] = useState('')
  const [workflowDescription, setWorkflowDescription] = useState('')
  const [yamlOutput, setYamlOutput] = useState('')

  const onConnect = useCallback(
    (connection: Connection) => {
      setEdges((eds) => addEdge({ ...connection, animated: true }, eds))
    },
    [setEdges]
  )

  const onNodeClick = useCallback((_: React.MouseEvent, node: Node) => {
    setSelectedNode(node)
  }, [])

  const addNode = (type: string, label: string, color: string) => {
    const id = `${type}_${Date.now()}`
    const newNode: Node = {
      id,
      type: 'default',
      position: { x: Math.random() * 300 + 100, y: Math.random() * 200 + 100 },
      data: { 
        label, 
        type,
        color,
        config: {}
      },
      style: {
        background: color,
        color: 'white',
        border: 'none',
        borderRadius: '8px',
        padding: '10px 20px',
        fontWeight: 600,
      },
    }
    setNodes((nds) => [...nds, newNode])
  }

  const generateYaml = () => {
    const workflowNodes: WorkflowNode[] = nodes.map((node) => {
      const dependsOn = edges
        .filter((edge) => edge.target === node.id)
        .map((edge) => edge.source.split('_')[0])
      
      return {
        id: node.id.split('_')[0],
        type: node.data.type,
        config: node.data.config || {},
        depends_on: dependsOn,
      }
    })

    const workflow: Workflow = {
      name: workflowName || 'new-workflow',
      description: workflowDescription || '',
      nodes: workflowNodes,
    }

    const yaml = `# ${workflow.description || workflow.name}
name: ${workflow.name}
description: ${workflow.description}
version: 1

nodes:
${workflowNodes.map((n) => `  - id: ${n.id}
    type: ${n.type}
    config:
      ${Object.entries(n.config).map(([k, v]) => `${k}: ${v}`).join('\n      ') || '# Add configuration'}
    ${n.depends_on.length > 0 ? `depends_on: [${n.depends_on.join(', ')}]` : ''}`).join('\n')}
`
    setYamlOutput(yaml)
  }

  const saveWorkflow = async () => {
    if (!workflowName) {
      alert('Please enter a workflow name')
      return
    }

    try {
      const response = await fetch(`/api/workflows/${workflowName}/execute`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      })
      
      if (response.ok) {
        alert('Workflow saved successfully!')
      } else {
        alert('Failed to save workflow')
      }
    } catch (err) {
      alert('Error saving workflow: ' + err)
    }
  }

  return (
    <div className="h-screen flex flex-col">
      {/* Header */}
      <div className="bg-white border-b border-slate-200 px-6 py-4">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-4">
            <h2 className="text-lg font-semibold text-slate-900">Visual Workflow Editor</h2>
            <input
              type="text"
              placeholder="Workflow name"
              value={workflowName}
              onChange={(e) => setWorkflowName(e.target.value)}
              className="px-3 py-1.5 border border-slate-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500"
            />
            <input
              type="text"
              placeholder="Description"
              value={workflowDescription}
              onChange={(e) => setWorkflowDescription(e.target.value)}
              className="px-3 py-1.5 border border-slate-300 rounded-md text-sm focus:outline-none focus:ring-2 focus:ring-indigo-500 w-64"
            />
          </div>
          <div className="flex gap-2">
            <button
              onClick={generateYaml}
              className="px-4 py-2 bg-slate-100 text-slate-700 rounded-md text-sm font-medium hover:bg-slate-200 transition-colors"
            >
              Generate YAML
            </button>
            <button
              onClick={saveWorkflow}
              className="px-4 py-2 bg-indigo-600 text-white rounded-md text-sm font-medium hover:bg-indigo-700 transition-colors"
            >
              Save Workflow
            </button>
          </div>
        </div>
      </div>

      <div className="flex-1 flex">
        {/* Node Palette */}
        <div className="w-64 bg-white border-r border-slate-200 p-4 overflow-y-auto">
          <h3 className="text-sm font-semibold text-slate-900 mb-4">Node Types</h3>
          <div className="space-y-2">
            {nodeTypes.map((nodeType) => (
              <button
                key={nodeType.type}
                onClick={() => addNode(nodeType.type, nodeType.label, nodeType.color)}
                className="w-full flex items-center gap-3 px-3 py-2 rounded-md hover:bg-slate-50 transition-colors text-left"
              >
                <div
                  className="w-3 h-3 rounded-full"
                  style={{ backgroundColor: nodeType.color }}
                />
                <span className="text-sm text-slate-700">{nodeType.label}</span>
              </button>
            ))}
          </div>
        </div>

        {/* Canvas */}
        <div className="flex-1 bg-slate-50">
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            onConnect={onConnect}
            onNodeClick={onNodeClick}
            fitView
          >
            <Background />
            <Controls />
            <MiniMap />
            <Panel position="top-left" className="bg-white p-2 rounded-lg shadow-sm">
              <p className="text-xs text-slate-500">Drag to connect nodes</p>
            </Panel>
          </ReactFlow>
        </div>

        {/* Properties Panel */}
        {selectedNode && (
          <div className="w-80 bg-white border-l border-slate-200 p-4">
            <h3 className="text-sm font-semibold text-slate-900 mb-4">Node Properties</h3>
            <div className="space-y-4">
              <div>
                <label className="block text-xs font-medium text-slate-700 mb-1">
                  Node ID
                </label>
                <input
                  type="text"
                  value={selectedNode.id}
                  readOnly
                  className="w-full px-3 py-2 bg-slate-100 border border-slate-200 rounded-md text-sm"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-slate-700 mb-1">
                  Node Type
                </label>
                <input
                  type="text"
                  value={selectedNode.data.type}
                  readOnly
                  className="w-full px-3 py-2 bg-slate-100 border border-slate-200 rounded-md text-sm"
                />
              </div>
              <div>
                <label className="block text-xs font-medium text-slate-700 mb-1">
                  Configuration (JSON)
                </label>
                <textarea
                  value={JSON.stringify(selectedNode.data.config || {}, null, 2)}
                  onChange={(e) => {
                    try {
                      const config = JSON.parse(e.target.value)
                      setNodes((nds) =>
                        nds.map((n) =>
                          n.id === selectedNode.id
                            ? { ...n, data: { ...n.data, config } }
                            : n
                        )
                      )
                    } catch {
                      // Invalid JSON, ignore
                    }
                  }}
                  rows={8}
                  className="w-full px-3 py-2 border border-slate-300 rounded-md text-sm font-mono focus:outline-none focus:ring-2 focus:ring-indigo-500"
                />
              </div>
              <button
                onClick={() => {
                  setNodes((nds) => nds.filter((n) => n.id !== selectedNode.id))
                  setEdges((eds) => eds.filter((e) => e.source !== selectedNode.id && e.target !== selectedNode.id))
                  setSelectedNode(null)
                }}
                className="w-full px-4 py-2 bg-red-50 text-red-600 rounded-md text-sm font-medium hover:bg-red-100 transition-colors"
              >
                Delete Node
              </button>
            </div>
          </div>
        )}
      </div>

      {/* YAML Output Modal */}
      {yamlOutput && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white rounded-lg shadow-xl max-w-2xl w-full mx-4 max-h-[80vh] flex flex-col">
            <div className="flex items-center justify-between px-6 py-4 border-b border-slate-200">
              <h3 className="text-lg font-semibold text-slate-900">Generated YAML</h3>
              <button
                onClick={() => setYamlOutput('')}
                className="text-slate-400 hover:text-slate-600"
              >
                ✕
              </button>
            </div>
            <div className="p-6 overflow-auto">
              <pre className="bg-slate-900 text-slate-50 p-4 rounded-lg text-sm overflow-x-auto">
                {yamlOutput}
              </pre>
            </div>
            <div className="flex justify-end gap-2 px-6 py-4 border-t border-slate-200">
              <button
                onClick={() => {
                  navigator.clipboard.writeText(yamlOutput)
                  alert('Copied to clipboard!')
                }}
                className="px-4 py-2 bg-slate-100 text-slate-700 rounded-md text-sm font-medium hover:bg-slate-200 transition-colors"
              >
                Copy to Clipboard
              </button>
              <button
                onClick={() => setYamlOutput('')}
                className="px-4 py-2 bg-indigo-600 text-white rounded-md text-sm font-medium hover:bg-indigo-700 transition-colors"
              >
                Close
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}
