export const MAX_THEMES = 10

export function limitReached(count: number): boolean {
  return count >= MAX_THEMES
}

export function limitMessage(count: number): string {
  return limitReached(count)
    ? `This realm holds ${MAX_THEMES} themes, the most it can. Delete one to make room.`
    : `${count} of ${MAX_THEMES} themes.`
}
