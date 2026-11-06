import type { SVGProps } from 'react'

/** The deskhaven mark (assets/logo/mark.svg), drawn in the current text colour. */
export function LogoMark(props: SVGProps<SVGSVGElement>) {
  return (
    <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100" {...props}>
      <path
        d="M26 20 H74 A8 8 0 0 1 82 28 V64 A8 8 0 0 1 74 72 H63 A13 13 0 0 0 37 72 H26 A8 8 0 0 1 18 64 V28 A8 8 0 0 1 26 20 Z"
        fill="currentColor"
        stroke="currentColor"
        strokeWidth="3"
        strokeLinejoin="round"
      />
      <circle cx="50" cy="71" r="5.5" fill="currentColor" />
      <path
        d="M37 86 H63"
        fill="none"
        stroke="currentColor"
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth="6"
      />
    </svg>
  )
}
