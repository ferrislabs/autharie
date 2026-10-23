import '@xyflow/react/dist/style.css'
import {
  Background,
  Controls,
  ReactFlow,
  ReactFlowProvider,
  useEdgesState,
  useNodesState,
  useReactFlow,
  type Connection,
  type FinalConnectionState,
  type Edge,
  type NodeMouseHandler,
} from '@xyflow/react'
import { FileText, MousePointerClick } from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import { Button } from '@/components/ui/button'
import {
  addBox,
  addLink,
  addTemplate,
  canLink,
  COLUMN_X,
  duplicateRule,
  focusOf,
  moveBoxes,
  newRuleBox,
  newWhatBox,
  newWhoBox,
  removeBox,
  removeLink,
  ruleOf,
  updateBox,
  type Box,
  type Point,
} from '../../../canvas-model'
import { summarisePolicy } from '../../../summary'
import type { AgentPolicy } from '../../../types'
import { LinkEdge } from './canvas-edge'
import { CanvasContextMenu, type MenuItem } from './canvas-context-menu'
import { CanvasUiContext, type CanvasUi, type RuleStatus } from './canvas-context'
import { CanvasLegend } from './canvas-legend'
import { HeaderNode, RuleNode, WhatNode, WhoNode } from './canvas-nodes'
import { DraftBar } from './draft-bar'
import { EditorLayer } from './editor-layer'
import {
  canvasHeight,
  mergeEdges,
  mergeNodes,
  type CanvasEdge,
  type CanvasNode,
} from './flow-elements'
import { boxMenu, linkMenu, paneMenu, templateItems, type AddActions, type Restrict } from './menu-items'
import { useColorMode } from './use-color-mode'
import { useCanvasDraft, type Publisher } from './use-canvas-draft'

interface Props {
  policies: AgentPolicy[]
  canManage: boolean
  publisher: Publisher
  onOpenAdvanced: (policy: AgentPolicy) => void
}

interface MenuState {
  x: number
  y: number
  label: string
  width: number
  height: number
  items: MenuItem[]
}

const NODE_TYPES = { who: WhoNode, rule: RuleNode, what: WhatNode, header: HeaderNode }
const EDGE_TYPES = { link: LinkEdge }
const TEXT_INPUTS = new Set(['INPUT', 'TEXTAREA', 'SELECT'])

function typing(target: EventTarget): boolean {
  return target instanceof HTMLElement && (TEXT_INPUTS.has(target.tagName) || target.isContentEditable)
}

function CustomCedar({ policies, onOpen }: { policies: AgentPolicy[]; onOpen: (policy: AgentPolicy) => void }) {
  return (
    <div className='space-y-2 rounded-lg border bg-card p-4'>
      <div>
        <p className='text-sm font-medium'>Custom Cedar policies</p>
        <p className='text-xs text-muted-foreground'>Edited in the advanced editor</p>
      </div>
      <ul className='divide-y'>
        {policies.map((policy) => (
          <li key={policy.id}>
            <button
              type='button'
              className='flex w-full items-center justify-between gap-3 py-2 text-left text-sm hover:text-primary'
              onClick={() => onOpen(policy)}
            >
              <span className='truncate font-medium'>{policy.name}</span>
              <span className='truncate text-xs text-muted-foreground'>{summarisePolicy(policy)}</span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  )
}

function CanvasEditor({ policies, canManage, publisher, onOpenAdvanced }: Props) {
  const readOnly = !canManage
  const colorMode = useColorMode()
  const { screenToFlowPosition, fitView } = useReactFlow()
  const draft = useCanvasDraft(policies, publisher)
  const { doc, pending } = draft
  const change = draft.edit

  const [initialNodes] = useState(() => mergeNodes(doc, []))
  const [initialEdges] = useState(() => mergeEdges(doc, []))
  const [nodes, setNodes, onNodesChange] = useNodesState<CanvasNode>(initialNodes)
  const [edges, setEdges, onEdgesChange] = useEdgesState<CanvasEdge>(initialEdges)
  const [syncedDoc, setSyncedDoc] = useState(doc)
  if (syncedDoc !== doc) {
    setSyncedDoc(doc)
    setNodes((previous) => mergeNodes(doc, previous))
    setEdges((previous) => mergeEdges(doc, previous))
  }

  const [hovered, setHovered] = useState<string | null>(null)
  const [editingId, setEditingId] = useState<string | null>(null)
  const [menu, setMenu] = useState<MenuState | null>(null)
  const wrapper = useRef<HTMLDivElement>(null)
  const fitted = useRef(doc.boxes.length > 0)

  useEffect(() => {
    if (fitted.current || doc.boxes.length === 0) return
    const timer = window.setTimeout(() => {
      fitted.current = true
      void fitView({ padding: 0.12, duration: 200 })
    }, 80)
    return () => window.clearTimeout(timer)
  }, [doc.boxes.length, fitView])

  const editing = editingId !== null && doc.boxes.some((box) => box.id === editingId) ? editingId : null
  const selectedId = nodes.find((node) => node.selected && node.type !== 'header')?.id ?? null
  const focusId = hovered ?? editing ?? selectedId
  const focus = useMemo(() => (focusId === null ? null : focusOf(doc, focusId)), [doc, focusId])

  const status = useMemo(() => {
    const result: Record<string, RuleStatus> = {}
    for (const box of doc.boxes) {
      if (box.kind !== 'rule') continue
      const outcome = ruleOf(doc, box.id)
      result[box.id] = {
        issues: outcome.ok ? [] : outcome.issues,
        changed: pending.changedRuleIds.has(box.id),
      }
    }
    return result
  }, [doc, pending])

  const ui = useMemo<CanvasUi>(() => ({ readOnly, focus, status }), [readOnly, focus, status])
  const height = useMemo(() => canvasHeight(doc), [doc])
  const unmapped = useMemo(() => policies.filter((policy) => !policy.rule), [policies])

  function containerPoint(clientX: number, clientY: number): Point {
    const rect = wrapper.current?.getBoundingClientRect()
    return { x: clientX - (rect?.left ?? 0), y: clientY - (rect?.top ?? 0) }
  }

  function openMenu(clientX: number, clientY: number, label: string, items: MenuItem[]) {
    const point = containerPoint(clientX, clientY)
    setEditingId(null)
    const box = wrapper.current
    setMenu({ ...point, label, items, width: box?.clientWidth ?? 1000, height: box?.clientHeight ?? 480 })
  }

  function addActions(flowPoint: Point, linkFrom: string | null): AddActions {
    const place = (box: Box, openEditor: boolean) => {
      const at = { x: COLUMN_X[box.kind], y: Math.max(0, Math.round(flowPoint.y - 28)) }
      change((current) => {
        const added = addBox(current, box, at)
        return linkFrom === null ? added : addLink(added, linkFrom, box.id)
      })
      setEditingId(openEditor ? box.id : null)
    }
    return {
      newRule: () => place(newRuleBox(doc), true),
      addWho: (subject, openEditor) => place(newWhoBox(doc, subject), openEditor),
      addWhat: (target, openEditor) => place(newWhatBox(doc, target), openEditor),
      fromTemplate: (template) => {
        change((current) => addTemplate(current, template, { x: 0, y: Math.max(0, Math.round(flowPoint.y - 40)) }))
        setEditingId(null)
      },
    }
  }

  function openPaneMenu(clientX: number, clientY: number, restrict: Restrict, linkFrom: string | null) {
    const flowPoint = screenToFlowPosition({ x: clientX, y: clientY })
    openMenu(clientX, clientY, 'Add to the canvas', paneMenu(addActions(flowPoint, linkFrom), restrict))
  }

  function removeBoxes(ids: string[], links: string[] = []) {
    change((current) => {
      const withoutLinks = links.reduce(removeLink, current)
      return ids.reduce(removeBox, withoutLinks)
    })
    if (editingId !== null && ids.includes(editingId)) setEditingId(null)
  }

  function focusRule(ruleId: string) {
    setEditingId(ruleId)
    setNodes((previous) => previous.map((node) => ({ ...node, selected: node.id === ruleId })))
    void fitView({ nodes: [{ id: ruleId }], padding: 1.2, maxZoom: 1, duration: 300 })
  }

  const onPaneContextMenu = (event: MouseEvent | React.MouseEvent) => {
    event.preventDefault()
    if (readOnly) return
    openPaneMenu(event.clientX, event.clientY, 'all', null)
  }

  const onNodeContextMenu: NodeMouseHandler<CanvasNode> = (event, node) => {
    event.preventDefault()
    if (readOnly) return
    if (node.type === 'header') {
      openPaneMenu(event.clientX, event.clientY, 'all', null)
      return
    }
    const box = doc.boxes.find((entry) => entry.id === node.id)
    if (!box) return
    const rule = box.kind === 'rule' ? box : null
    openMenu(
      event.clientX,
      event.clientY,
      'Box actions',
      boxMenu({
        edit: () => setEditingId(box.id),
        duplicate: rule ? () => change((current) => duplicateRule(current, box.id)) : undefined,
        toggle: rule
          ? {
              enabled: rule.enabled,
              run: () => change((current) => updateBox(current, { ...rule, enabled: !rule.enabled })),
            }
          : undefined,
        remove: () => removeBoxes([box.id]),
      }),
    )
  }

  const onEdgeContextMenu = (event: React.MouseEvent, edge: Edge) => {
    event.preventDefault()
    if (readOnly) return
    openMenu(event.clientX, event.clientY, 'Link actions', linkMenu(() => removeBoxes([], [edge.id])))
  }

  function onConnectEnd(event: MouseEvent | TouchEvent, state: FinalConnectionState) {
    if (readOnly || state.isValid || state.toNode !== null || state.fromNode === null) return
    const origin = 'changedTouches' in event ? event.changedTouches[0] : event
    const restrict: Restrict | null =
      state.fromNode.type === 'who' ? 'rule' : state.fromNode.type === 'rule' ? 'what' : null
    if (restrict === null) return
    openPaneMenu(origin.clientX, origin.clientY, restrict, state.fromNode.id)
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (event.key === 'Escape') {
      setMenu(null)
      setEditingId(null)
      return
    }
    if (readOnly || typing(event.target)) return
    if (event.key !== 'Delete' && event.key !== 'Backspace') return
    const ids = nodes.filter((node) => node.selected && node.type !== 'header').map((node) => node.id)
    const links = edges.filter((edge) => edge.selected).map((edge) => edge.id)
    if (ids.length === 0 && links.length === 0) return
    event.preventDefault()
    removeBoxes(ids, links)
  }

  function startFromTemplate(event: React.MouseEvent) {
    const rect = event.currentTarget.getBoundingClientRect()
    const point = screenToFlowPosition({ x: rect.left, y: rect.top })
    openMenu(rect.left, rect.bottom + 4, 'Templates', templateItems(addActions(point, null)))
  }

  return (
    <div className='space-y-4'>
      {canManage && (
        <DraftBar
          pending={pending}
          progress={draft.progress}
          error={draft.error}
          serverChanged={draft.serverChanged}
          onDiscard={draft.discard}
          onPublish={() => void draft.publish()}
          onFocusRule={focusRule}
        />
      )}

      <div
        ref={wrapper}
        tabIndex={-1}
        className='relative w-full overflow-hidden rounded-lg border bg-background outline-none'
        style={{ height }}
        onKeyDown={onKeyDown}
      >
        <CanvasUiContext.Provider value={ui}>
          <ReactFlow
            nodes={nodes}
            edges={edges}
            onNodesChange={onNodesChange}
            onEdgesChange={onEdgesChange}
            nodeTypes={NODE_TYPES}
            edgeTypes={EDGE_TYPES}
            colorMode={colorMode}
            fitView
            fitViewOptions={{ padding: 0.12 }}
            minZoom={0.3}
            maxZoom={1.5}
            nodesDraggable={!readOnly}
            nodesConnectable={!readOnly}
            elementsSelectable
            zoomOnScroll={false}
            zoomOnDoubleClick={false}
            preventScrolling={false}
            deleteKeyCode={null}
            isValidConnection={(connection: Connection | Edge) => canLink(doc, connection.source, connection.target)}
            onConnect={(connection) => change((current) => addLink(current, connection.source, connection.target))}
            onConnectEnd={onConnectEnd}
            onNodeClick={(_, node) => {
              setMenu(null)
              setEditingId(node.type === 'header' ? null : node.id)
            }}
            onPaneClick={() => {
              setMenu(null)
              setEditingId(null)
            }}
            onPaneContextMenu={onPaneContextMenu}
            onNodeContextMenu={onNodeContextMenu}
            onEdgeContextMenu={onEdgeContextMenu}
            onNodeMouseEnter={(_, node) => node.type !== 'header' && setHovered(node.id)}
            onNodeMouseLeave={() => setHovered(null)}
            onNodeDragStop={(_, __, dragged) =>
              change((current) =>
                moveBoxes(
                  current,
                  Object.fromEntries(dragged.filter((node) => node.type !== 'header').map((node) => [node.id, node.position])),
                ),
              )
            }
          >
            <Background gap={20} />
            <Controls showInteractive={false} />
            <EditorLayer
              doc={doc}
              editingId={editing}
              policies={policies}
              onChange={(box) => change((current) => updateBox(current, box))}
              onClose={() => setEditingId(null)}
              onOpenAdvanced={onOpenAdvanced}
            />
          </ReactFlow>
        </CanvasUiContext.Provider>

        {doc.boxes.length === 0 && (
          <div className='pointer-events-none absolute inset-0 z-10 flex flex-col items-center justify-center gap-3 text-center'>
            <MousePointerClick className='size-6 text-muted-foreground' />
            <p className='text-sm text-muted-foreground'>
              {canManage ? 'Right-click anywhere to add your first rule' : 'No access rules yet'}
            </p>
            {canManage && (
              <Button type='button' variant='outline' size='sm' className='pointer-events-auto' onClick={startFromTemplate}>
                <FileText className='size-4' />
                Start from a template
              </Button>
            )}
          </div>
        )}

        {menu && (
          <CanvasContextMenu
            x={menu.x}
            y={menu.y}
            items={menu.items}
            label={menu.label}
            width={menu.width}
            height={menu.height}
            onClose={() => setMenu(null)}
          />
        )}
      </div>

      <CanvasLegend canManage={canManage} />

      {unmapped.length > 0 && <CustomCedar policies={unmapped} onOpen={onOpenAdvanced} />}
    </div>
  )
}

export function PolicyCanvas(props: Props) {
  return (
    <ReactFlowProvider>
      <CanvasEditor {...props} />
    </ReactFlowProvider>
  )
}
