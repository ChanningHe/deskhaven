import { createFileRoute, notFound } from '@tanstack/react-router'

// A pathless layout never claims unmatched paths: without this splat they would fall through to
// the shell-less root not-found page.
export const Route = createFileRoute('/_app/$')({
  beforeLoad: () => {
    // notFound() returns a signal object, not an Error, and must be thrown.
    // eslint-disable-next-line @typescript-eslint/only-throw-error
    throw notFound({ routeId: '/_app' })
  },
})
