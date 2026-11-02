// Sign-in, sign-out and account setup requests (M3-02). The generated client
// makes every operation a query (orval.config.ts); state-changing calls are
// wrapped in mutations here. Whoever signs in or out drops the whole query
// cache first, so nothing fetched under one session leaks into the next.

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'
import type { QueryClient } from '@tanstack/react-query'
import { useNavigate } from '@tanstack/react-router'

import {
  completeOidcLogin,
  listOidcProviders,
  login,
  loginTotp,
  logout,
  startOidcLogin,
} from '@/api/generated/auth/auth'
import type {
  LoginRequest,
  Me,
  OidcCompleteRequest,
  OidcStartRequest,
  TotpLoginRequest,
} from '@/api/generated/model'
import { getSetupStatus } from '@/api/generated/setup/setup'
import { assignLocation } from '@/lib/browser-location'

import { LOGIN_PATH, ME_QUERY_KEY } from './session'

/** A new session exists: forget everything cached, keep its user. */
export function acceptSession(queryClient: QueryClient, me: Me): void {
  queryClient.clear()
  queryClient.setQueryData(ME_QUERY_KEY, me)
}

export function useLogin() {
  return useMutation({ mutationFn: (body: LoginRequest) => login(body) })
}

export function useTotpLogin() {
  return useMutation({ mutationFn: (body: TotpLoginRequest) => loginTotp(body) })
}

export function useOidcProviders() {
  return useQuery({
    queryKey: ['auth', 'oidc-providers'],
    queryFn: ({ signal }) => listOidcProviders({ signal }),
  })
}

/** Starts an OIDC login and leaves for the identity provider. */
export function useStartOidc() {
  return useMutation({
    mutationFn: (body: OidcStartRequest) => startOidcLogin(body),
    onSuccess: ({ url }) => {
      assignLocation(url)
    },
  })
}

export function useCompleteOidc() {
  return useMutation({ mutationFn: (body: OidcCompleteRequest) => completeOidcLogin(body) })
}

export function useSetupStatus() {
  return useQuery({
    queryKey: ['setup', 'status'],
    queryFn: ({ signal }) => getSetupStatus({ signal }),
  })
}

/** Signs out (the server clears the cookies), goes to /login, drops the cache. */
export function useLogout() {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  return useMutation({
    mutationFn: () => logout(),
    onSettled: async () => {
      // Leave the console first: its mounted queries would refetch a cleared `me`.
      await navigate({ to: LOGIN_PATH, replace: true })
      queryClient.clear()
    },
  })
}
