import { useMemo, useState } from 'react'
import {
  docFromPolicies,
  keepPositions,
  pendingChanges,
  updateBox,
  type CanvasDoc,
  type Pending,
  type RuleBox,
} from '../../../canvas-model'
import type { AgentPolicy, AgentPolicyInput } from '../../../types'

export interface Publisher {
  create: (input: AgentPolicyInput) => Promise<{ id: string }>
  update: (id: string, input: AgentPolicyInput) => Promise<unknown>
  remove: (id: string) => Promise<unknown>
}

export interface Progress {
  done: number
  total: number
}

export interface CanvasDraft {
  doc: CanvasDoc
  edit: (change: (doc: CanvasDoc) => CanvasDoc) => void
  pending: Pending
  serverChanged: boolean
  progress: Progress | null
  error: string | null
  discard: () => void
  publish: () => Promise<void>
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : 'Publishing failed.'
}

export function useCanvasDraft(policies: AgentPolicy[], publisher: Publisher): CanvasDraft {
  const [doc, setDoc] = useState(() => docFromPolicies(policies))
  const [base, setBase] = useState(policies)
  const [serverChanged, setServerChanged] = useState(false)
  const [follow, setFollow] = useState(false)
  const [progress, setProgress] = useState<Progress | null>(null)
  const [error, setError] = useState<string | null>(null)

  const pending = useMemo(() => pendingChanges(policies, doc), [policies, doc])

  if (policies !== base || follow) {
    const local = !follow && pendingChanges(base, doc).count > 0
    if (!local) {
      setBase(policies)
      setDoc(keepPositions(docFromPolicies(policies), doc))
      setServerChanged(false)
      setFollow(false)
    } else if (!serverChanged) {
      setServerChanged(true)
    }
  }

  function discard() {
    setBase(policies)
    setDoc(keepPositions(docFromPolicies(policies), doc))
    setServerChanged(false)
    setError(null)
  }

  async function publish() {
    const { diff } = pending
    const total = diff.create.length + diff.update.length + diff.delete.length
    let done = 0
    setError(null)
    setProgress({ done, total })
    try {
      for (let index = 0; index < diff.create.length; index += 1) {
        const created = await publisher.create(diff.create[index])
        const ruleId = diff.createIds[index]
        setDoc((current) => {
          const box = current.boxes.find((entry) => entry.id === ruleId)
          return box?.kind === 'rule' ? updateBox(current, { ...box, policyId: created.id } satisfies RuleBox) : current
        })
        done += 1
        setProgress({ done, total })
      }
      for (const entry of diff.update) {
        await publisher.update(entry.id, entry.input)
        done += 1
        setProgress({ done, total })
      }
      for (const id of diff.delete) {
        await publisher.remove(id)
        done += 1
        setProgress({ done, total })
      }
      setFollow(true)
    } catch (failure) {
      setError(messageOf(failure))
    } finally {
      setProgress(null)
    }
  }

  return {
    doc,
    edit: (change) => setDoc(change),
    pending,
    serverChanged,
    progress,
    error,
    discard,
    publish,
  }
}
