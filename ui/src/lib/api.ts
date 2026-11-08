// API access for the dashboard.
//
// When the server runs with R8R_API_KEY, every /api/* call needs
// `Authorization: Bearer <key>`. The key is kept in sessionStorage, so it is
// forgotten when the tab closes, with an in-memory copy for when storage is
// unavailable (private windows, blocked site data).

const STORAGE_KEY = 'r8r.apiKey'
export const UNAUTHORIZED_EVENT = 'r8r:unauthorized'

let memoryKey: string | null = null

export function getApiKey(): string | null {
  try {
    return sessionStorage.getItem(STORAGE_KEY) ?? memoryKey
  } catch {
    return memoryKey
  }
}

export function setApiKey(key: string | null): void {
  memoryKey = key
  try {
    if (key) {
      sessionStorage.setItem(STORAGE_KEY, key)
    } else {
      sessionStorage.removeItem(STORAGE_KEY)
    }
  } catch {
    // Storage unavailable: the in-memory copy still works for this page load.
  }
}

export async function apiFetch(input: string, init: RequestInit = {}): Promise<Response> {
  const headers = new Headers(init.headers)
  const key = getApiKey()
  if (key) {
    headers.set('Authorization', `Bearer ${key}`)
  }
  const response = await fetch(input, { ...init, headers })
  if (response.status === 401) {
    window.dispatchEvent(new Event(UNAUTHORIZED_EVENT))
  }
  return response
}
