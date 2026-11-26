import { useRef } from 'react'
import type { RefCallback } from 'react'

const REDUCED_MOTION = '(prefers-reduced-motion: reduce)'

function scrollBehavior(): ScrollBehavior {
  return window.matchMedia(REDUCED_MOTION).matches ? 'auto' : 'smooth'
}

/** Moves focus to the heading so a screen reader continues from there; the URL is left alone. */
export function useCategoryJump() {
  const headings = useRef(new Map<string, HTMLHeadingElement>())

  const register =
    (name: string): RefCallback<HTMLHeadingElement> =>
    (heading) => {
      if (heading === null) return
      headings.current.set(name, heading)
      return () => {
        headings.current.delete(name)
      }
    }

  const jump = (name: string) => {
    const heading = headings.current.get(name)
    if (heading === undefined) return
    heading.scrollIntoView({ block: 'start', behavior: scrollBehavior() })
    heading.focus({ preventScroll: true })
  }

  return { register, jump }
}
