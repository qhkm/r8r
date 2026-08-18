// Workflow definition ↔ visual canvas converters.
// Mirrors src/workflow/types.rs (Trigger / Node schema).
import * as YAML from 'js-yaml'
import type { Edge, Node } from 'reactflow'

export interface WorkflowTrigger {
  type: 'manual' | 'cron' | 'webhook' | 'event'
  schedule?: string
  timezone?: string
  path?: string
  method?: string
  event?: string
  filter?: string
  [k: string]: unknown
}

export interface WorkflowNodeDef {
  id: string
  type: string
  config: Record<string, unknown>
  depends_on?: string[]
}

export interface WorkflowDefinition {
  name: string
  description: string
  version: number
  triggers: WorkflowTrigger[]
  nodes: WorkflowNodeDef[]
}

export interface CanvasNodeData {
  label: string
  nodeType: string
  color: string
  config: Record<string, unknown>
}

export const NODE_COLORS: Record<string, string> = {
  http: '#2563eb',
  transform: '#7c3aed',
  agent: '#d97706',
  if: '#dc2626',
  switch: '#ea580c',
  merge: '#0891b2',
  filter: '#059669',
  sort: '#0d9488',
  limit: '#0f766e',
  set: '#4338ca',
  aggregate: '#4f46e5',
  split: '#65a30d',
  wait: '#475569',
  crypto: '#6d28d9',
  datetime: '#9333ea',
  dedupe: '#0e7490',
  subworkflow: '#1e40af',
  variables: '#3730a3',
  debug: '#6d28d9',
  summarize: '#b45309',
  email: '#0369a1',
  slack: '#7c3aed',
  telegram: '#0284c7',
  'google-sheets': '#15803d',
  discord: '#4c1d95',
  database: '#1d4ed8',
  s3: '#b45309',
  approval: '#0f766e',
  integration: '#be123c',
  document: '#78350f',
}

export const NODE_LIBRARY: Array<{ nodeType: string; label: string; group: string }> = [
  // Core
  { nodeType: 'http', label: 'HTTP Request', group: 'Core' },
  { nodeType: 'transform', label: 'Transform', group: 'Core' },
  { nodeType: 'if', label: 'Condition (if)', group: 'Core' },
  { nodeType: 'switch', label: 'Switch', group: 'Core' },
  { nodeType: 'merge', label: 'Merge', group: 'Core' },
  { nodeType: 'split', label: 'Split', group: 'Core' },
  { nodeType: 'set', label: 'Set Variables', group: 'Core' },
  { nodeType: 'variables', label: 'Variables', group: 'Core' },
  { nodeType: 'wait', label: 'Wait', group: 'Core' },
  { nodeType: 'approval', label: 'Approval Gate', group: 'Core' },
  { nodeType: 'subworkflow', label: 'SubWorkflow', group: 'Core' },
  { nodeType: 'debug', label: 'Debug', group: 'Core' },
  // Data processing
  { nodeType: 'filter', label: 'Filter', group: 'Data' },
  { nodeType: 'sort', label: 'Sort', group: 'Data' },
  { nodeType: 'limit', label: 'Limit', group: 'Data' },
  { nodeType: 'dedupe', label: 'Dedupe', group: 'Data' },
  { nodeType: 'aggregate', label: 'Aggregate', group: 'Data' },
  { nodeType: 'datetime', label: 'DateTime', group: 'Data' },
  { nodeType: 'crypto', label: 'Crypto', group: 'Data' },
  { nodeType: 'document', label: 'Document (PDF/OCR)', group: 'Data' },
  // AI
  { nodeType: 'agent', label: 'AI Agent', group: 'AI' },
  { nodeType: 'summarize', label: 'Summarize', group: 'AI' },
  // Communications
  { nodeType: 'email', label: 'Email', group: 'Comms' },
  { nodeType: 'slack', label: 'Slack', group: 'Comms' },
  { nodeType: 'telegram', label: 'Telegram', group: 'Comms' },
  { nodeType: 'discord', label: 'Discord', group: 'Comms' },
  // Data stores
  { nodeType: 'google-sheets', label: 'Google Sheets', group: 'Stores' },
  { nodeType: 'database', label: 'Database', group: 'Stores' },
  { nodeType: 's3', label: 'S3', group: 'Stores' },
  // Integrations
  { nodeType: 'integration', label: 'Integration Service', group: 'Integration' },
]

export function sanitizeWorkflowName(raw: string): string {
  const fallback = 'visual_workflow'
  const normalized = raw
    .trim()
    .toLowerCase()
    .replace(/\s+/g, '_')
    .replace(/[^a-z0-9_-]/g, '')
    .replace(/_+/g, '_')

  return normalized.length > 0 ? normalized : fallback
}

const EDGE_X_GAP = 220
const EDGE_Y_GAP = 90

/** Parse a stored workflow definition (YAML or JSON string) into ReactFlow state. */
export function parseDefinitionToCanvas(
  rawDefinition: string
): { name: string; description: string; workflowName: string; nodes: Node<CanvasNodeData>[]; edges: Edge[]; triggers: WorkflowTrigger[] } {
  let parsed: unknown
  try {
    parsed = YAML.load(rawDefinition)
  } catch {
    parsed = JSON.parse(rawDefinition)
  }

  const def = (parsed ?? {}) as Partial<WorkflowDefinition>
  const defNodes = def.nodes ?? []
  const defTriggers = def.triggers ?? []

  // Deterministic grid layout: group nodes by their graph depth (no deps = depth 0).
  const depthMap = new Map<string, number>()
  const getDepth = (nodeId: string): number => {
    const cached = depthMap.get(nodeId)
    if (cached !== undefined) return cached
    const node = defNodes.find((n) => n.id === nodeId)
    const deps = node?.depends_on ?? []
    const depth = deps.length === 0 ? 0 : 1 + Math.max(0, ...deps.map((d) => getDepth(d)))
    depthMap.set(nodeId, depth)
    return depth
  }

  const colCount = new Map<number, number>()
  const canvasNodes: Node<CanvasNodeData>[] = defNodes.map((node) => {
    const depth = getDepth(node.id)
    const col = colCount.get(depth) ?? 0
    colCount.set(depth, col + 1)
    return {
      id: node.id,
      position: { x: 40 + depth * EDGE_X_GAP, y: 40 + col * EDGE_Y_GAP },
      data: {
        label: nodeLabel(node),
        nodeType: node.type,
        color: NODE_COLORS[node.type] ?? '#64748b',
        config: (node.config ?? {}) as Record<string, unknown>,
      },
      style: {
        background: NODE_COLORS[node.type] ?? '#64748b',
        color: '#ffffff',
        border: 'none',
        borderRadius: '10px',
        padding: '10px 14px',
        fontWeight: 600,
      },
    }
  })

  const canvasEdges: Edge[] = []
  defNodes.forEach((node) => {
    for (const dep of node.depends_on ?? []) {
      canvasEdges.push({
        id: `${dep}->${node.id}`,
        source: dep,
        target: node.id,
        animated: true,
      })
    }
  })

  const rawName = def.name ?? 'visual_workflow'
  return {
    name: rawName,
    description: def.description ?? '',
    workflowName: sanitizeWorkflowName(rawName),
    nodes: canvasNodes,
    edges: canvasEdges,
    triggers: defTriggers,
  }
}

function nodeLabel(node: WorkflowNodeDef): string {
  const entry = NODE_LIBRARY.find((n) => n.nodeType === node.type)
  if (entry) return entry.label
  return node.type.charAt(0).toUpperCase() + node.type.slice(1)
}

/** Build the workflow definition object from current canvas state. */
export function buildDefinitionFromCanvas(
  workflowName: string,
  workflowDescription: string,
  nodes: Node<CanvasNodeData>[],
  edges: Edge[],
  triggers: WorkflowTrigger[]
): WorkflowDefinition {
  const workflowNodes: WorkflowNodeDef[] = nodes.map((node) => {
    const dependencies = edges
      .filter((edge) => edge.target === node.id)
      .map((edge) => edge.source)

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
    triggers: triggers.length > 0 ? triggers : [{ type: 'manual' }],
    nodes: workflowNodes,
  }
}

/** Serialize a definition to YAML for saving (server stores YAML; JSON parses fine as YAML subset). */
export function definitionToYaml(definition: WorkflowDefinition): string {
  return YAML.dump(definition, { noRefs: true })
}
