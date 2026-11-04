import type { BookView, PeerView, ShareView, TagView } from '@/api/generated/model'
import type { MockHandler } from '@/test/mock-api'

export const BOOKS = '/admin/api/v1/address-books'

export function book(guid: string, name: string, patch: Partial<BookView> = {}): BookView {
  return {
    guid,
    name,
    note: '',
    owner_user_id: null,
    owner: null,
    peer_count: 0,
    tag_count: 0,
    share_count: 0,
    created_at: 1_700_000_000_000,
    updated_at: 1_700_000_000_000,
    ...patch,
  }
}

export function peer(id: string, patch: Partial<PeerView> = {}): PeerView {
  return {
    id,
    alias: '',
    note: '',
    username: '',
    hostname: '',
    platform: '',
    rdp_port: '',
    rdp_username: '',
    login_name: '',
    tags: [],
    force_always_relay: false,
    has_password: false,
    ...patch,
  }
}

export function share(
  kind: ShareView['subject_type'],
  id: number,
  name: string,
  rule: number,
): ShareView {
  return { subject_type: kind, subject_id: id, subject_name: name, rule }
}

export function tag(name: string, color = 0xff9e9e9e): TagView {
  return { name, color }
}

/** A paged list response. */
export function page(items: unknown[]): MockHandler {
  return () => ({ body: { items, total: items.length, page: 1, page_size: 20 } })
}

/** A non-paged `{items}` response. */
export function items(list: unknown[]): MockHandler {
  return () => ({ body: { items: list } })
}
