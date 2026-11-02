import { createFileRoute, notFound } from '@tanstack/react-router'

// Catch-all for unknown console URLs. A pathless layout never claims unmatched
// paths by itself, so without this splat they would fall through to the root
// (shell-less) not-found page. The notFound() is routed explicitly to the
// `_app` layout, whose notFoundComponent renders inside the sidebar shell.
export const Route = createFileRoute('/_app/$')({
  beforeLoad: () => {
    // TanStack Router control flow: notFound() returns a typed signal object,
    // not an Error, and must be thrown.
    // eslint-disable-next-line @typescript-eslint/only-throw-error
    throw notFound({ routeId: '/_app' })
  },
})
