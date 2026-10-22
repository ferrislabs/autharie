import '@xyflow/react/dist/style.css'
import {
  Background,
  BaseEdge,
  Controls,
  EdgeLabelRenderer,
  getBezierPath,
  Handle,
  MarkerType,
  Panel,
  Position,
  ReactFlow,
  type ColorMode,
  type Connection,
  type Edge,
  type EdgeProps,
  type Node,
  type NodeProps,
} from '@xyflow/react'
import {
  AlertTriangle,
  Bot,
  Clock,
  Eye,
  Globe,
  KeyRound,
  Map as MapIcon,
  Plus,
  Server,
  User,
  Users,
  Wrench,
} from 'lucide-react'
import { useEffect, useMemo, useState } from 'react'
import { EmptyState } from '@/components/layout/page'
import { Button } from '@/components/ui/button'
import { cn } from '@/lib/utils'
import {
  buildAccessGraph,
  COLUMN_X,
  connectedPolicies,
  ruleFromConnection,
  type AccessEdgeData,
  type AccessNodeData,
  type Column,
} from '../../access-map'
import { summarisePolicy } from '../../summary'
import type { AgentPolicy, RuleModel } from '../../types'

interface Props {
  policies: AgentPolicy[]
  canManage: boolean
  highlightPolicyIds?: string[]
  onEditPolicy: (policy: AgentPolicy) => void
  onCreateRule: (rule: RuleModel) => void
  onCreatePolicy: () => void
}

type MapNodeData = AccessNodeData & { canManage: boolean }
type HeaderData = { title: string; hint: string }
type MapEdgeData = AccessEdgeData & { dimmed: boolean }

type MapNode = Node<MapNodeData, Column> | Node<HeaderData, 'header'>
type MapEdge = Edge<MapEdgeData, 'access'>

const PERMIT = '#10b981'
const FORBID = 'var(--destructive)'
const MUTED = 'var(--muted-foreground)'
const NODE_WIDTH = 240
const HEADER_Y = -76

const HEADERS: Record<Column, HeaderData> = {
  who: { title: 'Who', hint: 'People asking' },
  agent: { title: 'Through which agent', hint: 'The AI tool they use' },
  what: { title: 'What', hint: 'Tools they may call' },
}

function strokeOf(data: AccessEdgeData): string {
  if (!data.enabled) return MUTED
  return data.effect === 'permit' ? PERMIT : FORBID
}

function NodeIcon({ id }: { id: string }) {
  const className = 'size-4'
  if (id === 'who:any') return <Globe className={className} />
  if (id.startsWith('who:group:')) return <Users className={className} />
  if (id.startsWith('who:role:')) return <KeyRound className={className} />
  if (id.startsWith('who:user:')) return <User className={className} />
  if (id.startsWith('agent:')) return <Bot className={className} />
  if (id.startsWith('what:read-only:')) return <Eye className={className} />
  if (id.startsWith('what:server:')) return <Server className={className} />
  return <Wrench className={className} />
}

function useColorMode(): ColorMode {
  const read = () => (document.documentElement.classList.contains('dark') ? 'dark' : 'light')
  const [mode, setMode] = useState<ColorMode>(read)

  useEffect(() => {
    const observer = new MutationObserver(() => setMode(read()))
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['class'] })
    return () => observer.disconnect()
  }, [])

  return mode
}

const HANDLE = 'size-2.5! border-2! border-background! bg-muted-foreground!'

function AccessNodeCard({ id, data, left, right }: { id: string; data: MapNodeData; left: boolean; right: boolean }) {
  const autonomous = data.caption === 'acts on its own'
  const handle = cn(HANDLE, data.canManage && 'bg-primary!')

  return (
    <div
      className={cn(
        'flex items-center gap-3 rounded-lg border bg-card px-3 py-2 text-card-foreground shadow-xs',
        data.destructive && 'border-destructive/50',
      )}
      style={{ width: NODE_WIDTH }}
    >
      {left && <Handle type='target' position={Position.Left} className={HANDLE} isConnectable={false} />}
      <div
        className={cn(
          'flex size-8 shrink-0 items-center justify-center rounded-md border bg-muted text-muted-foreground',
          data.destructive && 'border-destructive/30 bg-destructive/10 text-destructive',
          autonomous && 'border-primary/30 bg-primary/10 text-primary',
        )}
      >
        <NodeIcon id={id} />
      </div>
      <div className='min-w-0 flex-1'>
        <p className='truncate text-sm font-medium' title={data.label}>
          {data.label}
        </p>
        <p className='flex items-center gap-1 truncate text-xs text-muted-foreground'>
          {data.destructive && <AlertTriangle className='size-3 shrink-0 text-destructive' />}
          <span className='truncate'>{data.destructive ? 'destructive, ' : ''}{data.caption}</span>
        </p>
      </div>
      {right && <Handle type='source' position={Position.Right} className={handle} />}
    </div>
  )
}

function WhoNode({ id, data }: NodeProps<Node<MapNodeData, 'who'>>) {
  return <AccessNodeCard id={id} data={data} left={false} right />
}

function AgentNode({ id, data }: NodeProps<Node<MapNodeData, 'agent'>>) {
  return <AccessNodeCard id={id} data={data} left right />
}

function WhatNode({ id, data }: NodeProps<Node<MapNodeData, 'what'>>) {
  return <AccessNodeCard id={id} data={data} left right={false} />
}

function HeaderNode({ data }: NodeProps<Node<HeaderData, 'header'>>) {
  return (
    <div style={{ width: NODE_WIDTH }} className='px-1'>
      <p className='text-xs font-semibold uppercase tracking-wide text-foreground'>{data.title}</p>
      <p className='text-xs text-muted-foreground'>{data.hint}</p>
    </div>
  )
}

function AccessEdgeView({
  id,
  sourceX,
  sourceY,
  targetX,
  targetY,
  sourcePosition,
  targetPosition,
  markerEnd,
  style,
  data,
  selected,
}: EdgeProps<MapEdge>) {
  const [path, labelX, labelY] = getBezierPath({
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
  })
  const color = data ? strokeOf(data) : MUTED

  return (
    <>
      <BaseEdge id={id} path={path} markerEnd={markerEnd} style={style} interactionWidth={24} />
      {data?.label && (
        <EdgeLabelRenderer>
          <div
            className={cn(
              'nodrag nopan pointer-events-none absolute flex items-center gap-1 rounded-full border bg-card px-2 py-0.5 text-[11px] text-card-foreground transition-opacity',
              selected && 'ring-2 ring-ring',
            )}
            style={{
              transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
              borderColor: color,
              opacity: data.dimmed ? 0.2 : 1,
            }}
          >
            <Clock className='size-3' />
            {data.label}
          </div>
        </EdgeLabelRenderer>
      )}
    </>
  )
}

const NODE_TYPES = {
  who: WhoNode,
  agent: AgentNode,
  what: WhatNode,
  header: HeaderNode,
}

const EDGE_TYPES = { access: AccessEdgeView }

function LegendItem({ children, line }: { children: React.ReactNode; line: React.ReactNode }) {
  return (
    <span className='flex items-center gap-1.5'>
      {line}
      {children}
    </span>
  )
}

function Legend() {
  const swatch = (color: string, dashed = false) => (
    <svg width='22' height='6' aria-hidden>
      <line
        x1='0'
        y1='3'
        x2='22'
        y2='3'
        stroke={color}
        strokeWidth='2.5'
        strokeDasharray={dashed ? '4 3' : undefined}
      />
    </svg>
  )

  return (
    <div className='flex flex-wrap items-center gap-x-4 gap-y-1 rounded-md border bg-card/90 px-3 py-1.5 text-xs text-muted-foreground'>
      <LegendItem line={swatch(PERMIT)}>Permit</LegendItem>
      <LegendItem line={swatch(FORBID)}>Forbid</LegendItem>
      <LegendItem line={swatch(MUTED, true)}>Disabled</LegendItem>
      <LegendItem line={<Clock className='size-3' />}>Condition</LegendItem>
    </div>
  )
}

export function AccessMap({
  policies,
  canManage,
  highlightPolicyIds,
  onEditPolicy,
  onCreateRule,
  onCreatePolicy,
}: Props) {
  const colorMode = useColorMode()
  const graph = useMemo(() => buildAccessGraph(policies), [policies])
  const [hovered, setHovered] = useState<string | null>(null)
  const [selected, setSelected] = useState<string | null>(null)

  const focus = hovered ?? selected
  const active = useMemo(() => {
    if (highlightPolicyIds) return new Set(highlightPolicyIds)
    if (focus) return new Set(connectedPolicies(graph, focus))
    return null
  }, [graph, highlightPolicyIds, focus])

  const nodes = useMemo(() => {
    const touched = new Set<string>()
    if (active) {
      for (const edge of graph.edges) {
        if (active.has(edge.data.policyId)) {
          touched.add(edge.source)
          touched.add(edge.target)
        }
      }
    }
    const columns: Column[] = ['who', 'agent', 'what']
    const headers: MapNode[] = columns.map((column) => ({
      id: `header:${column}`,
      type: 'header',
      position: { x: COLUMN_X[column], y: HEADER_Y },
      data: HEADERS[column],
      draggable: false,
      selectable: false,
      focusable: false,
      connectable: false,
    }))
    const cards: MapNode[] = graph.nodes.map((node) => ({
      id: node.id,
      type: node.type,
      position: node.position,
      data: { ...node.data, canManage },
      draggable: false,
      className: 'transition-opacity',
      style: { opacity: active && !touched.has(node.id) ? 0.25 : 1 },
    })) as MapNode[]
    return [...headers, ...cards]
  }, [graph, active, canManage])

  const edges = useMemo<MapEdge[]>(
    () =>
      graph.edges.map((edge) => {
        const dimmed = active !== null && !active.has(edge.data.policyId)
        const color = strokeOf(edge.data)
        return {
          ...edge,
          data: { ...edge.data, dimmed },
          markerEnd: { type: MarkerType.ArrowClosed, color, width: 16, height: 16 },
          style: {
            stroke: color,
            strokeWidth: edge.data.enabled ? 2.5 : 2,
            strokeDasharray: edge.data.enabled ? undefined : '6 5',
            opacity: dimmed ? 0.15 : edge.data.enabled ? 1 : 0.7,
            cursor: 'pointer',
          },
        }
      }),
    [graph, active],
  )

  const structure = useMemo(() => graph.nodes.map((node) => node.id).join('|'), [graph])

  function isValidConnection(connection: Connection | Edge): boolean {
    return ruleFromConnection(connection.source, connection.target) !== null
  }

  function connect(connection: Connection) {
    const rule = ruleFromConnection(connection.source, connection.target)
    if (rule) onCreateRule(rule)
  }

  function editEdge(policyId: string) {
    const policy = policies.find((candidate) => candidate.id === policyId)
    if (policy) onEditPolicy(policy)
  }

  if (policies.length === 0) {
    return (
      <EmptyState
        icon={<MapIcon className='size-5' />}
        title='No access rules yet'
        description='An access rule says who may call which tools, through which AI agent. Create one and it shows up here as a map.'
        action={
          canManage && (
            <Button onClick={onCreatePolicy}>
              <Plus className='size-4' />
              Create your first rule
            </Button>
          )
        }
      />
    )
  }

  return (
    <div className='space-y-4'>
      {graph.nodes.length > 0 ? (
        <div className='h-[560px] w-full overflow-hidden rounded-lg border bg-background'>
          <ReactFlow
            key={structure}
            nodes={nodes}
            edges={edges}
            nodeTypes={NODE_TYPES}
            edgeTypes={EDGE_TYPES}
            colorMode={colorMode}
            fitView
            fitViewOptions={{ padding: 0.12 }}
            minZoom={0.3}
            nodesDraggable={false}
            nodesConnectable={canManage}
            elementsSelectable
            zoomOnScroll={false}
            preventScrolling={false}
            isValidConnection={isValidConnection}
            onConnect={connect}
            onEdgeClick={(_, edge) => editEdge(edge.data?.policyId ?? '')}
            onNodeMouseEnter={(_, node) => node.type !== 'header' && setHovered(node.id)}
            onNodeMouseLeave={() => setHovered(null)}
            onNodeClick={(_, node) => setSelected(node.type === 'header' ? null : node.id)}
            onPaneClick={() => setSelected(null)}
          >
            <Background gap={20} />
            <Controls showInteractive={false} />
            <Panel position='bottom-right'>
              <Legend />
            </Panel>
          </ReactFlow>
        </div>
      ) : (
        <p className='rounded-lg border bg-muted/30 px-4 py-6 text-center text-sm text-muted-foreground'>
          None of these policies can be drawn on the map.
        </p>
      )}

      {canManage && graph.nodes.length > 0 && (
        <p className='text-xs text-muted-foreground'>
          Click a line to edit its rule. Drag from a person or agent to a tool to start a new rule.
        </p>
      )}

      {graph.unmapped.length > 0 && (
        <div className='space-y-2 rounded-lg border bg-card p-4'>
          <p className='text-sm font-medium'>Custom Cedar policies — not drawn</p>
          <ul className='divide-y'>
            {graph.unmapped.map((policy) => (
              <li key={policy.id}>
                <button
                  type='button'
                  className='flex w-full items-center justify-between gap-3 py-2 text-left text-sm hover:text-primary'
                  onClick={() => onEditPolicy(policy)}
                >
                  <span className='truncate font-medium'>{policy.name}</span>
                  <span className='truncate text-xs text-muted-foreground'>{summarisePolicy(policy)}</span>
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  )
}
