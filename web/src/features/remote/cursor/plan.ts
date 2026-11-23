export interface CursorScene {
  embedded: boolean
  control: boolean
  showRemote: boolean
  hasShape: boolean
  css: string | null
}

export function localCursor(scene: CursorScene): string {
  if (scene.embedded || !scene.control || !scene.hasShape) return 'default'
  return scene.css ?? 'none'
}

export function paintedCursor(scene: CursorScene, remoteMoved: boolean): 'local' | 'remote' | null {
  if (scene.embedded) return null
  if (!scene.control) return 'remote'
  if (!scene.hasShape) return null
  if (remoteMoved && scene.showRemote) return 'remote'
  return scene.css === null ? 'local' : null
}
