import { format } from 'date-fns'

export function humanise(value: string): string {
  const spaced = value.replace(/[_-]+/g, ' ').trim()
  return spaced === '' ? value : spaced.charAt(0).toUpperCase() + spaced.slice(1)
}

export function formatWhen(iso: string): string {
  const date = new Date(iso)
  return Number.isNaN(date.getTime()) ? iso : format(date, 'yyyy-MM-dd HH:mm')
}

export function formatSeconds(seconds: number): string {
  return `${seconds} s`
}
