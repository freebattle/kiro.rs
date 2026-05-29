import type { BalanceResponse } from '@/types/api'

const API_KEY_STORAGE_KEY = 'adminApiKey'
const BALANCE_CACHE_STORAGE_KEY = 'credentialBalanceCache'
const CREDITS_CURSOR_STORAGE_KEY = 'creditsSyncCursor'

interface CreditsSyncCursor {
  lastTimestamp: number
  processedKeysAtTimestamp: string[]
}

function isBalanceResponse(value: unknown): value is BalanceResponse {
  if (!value || typeof value !== 'object') {
    return false
  }

  const candidate = value as Record<string, unknown>
  return (
    typeof candidate.id === 'number' &&
    (candidate.subscriptionTitle === null || typeof candidate.subscriptionTitle === 'string') &&
    typeof candidate.currentUsage === 'number' &&
    typeof candidate.usageLimit === 'number' &&
    typeof candidate.remaining === 'number' &&
    typeof candidate.usagePercentage === 'number' &&
    (candidate.nextResetAt === null || typeof candidate.nextResetAt === 'number')
  )
}

function parseJson<T>(raw: string | null): T | null {
  if (!raw) {
    return null
  }

  try {
    return JSON.parse(raw) as T
  } catch {
    return null
  }
}

export const storage = {
  getApiKey: () => localStorage.getItem(API_KEY_STORAGE_KEY),
  setApiKey: (key: string) => localStorage.setItem(API_KEY_STORAGE_KEY, key),
  removeApiKey: () => localStorage.removeItem(API_KEY_STORAGE_KEY),

  getBalanceCache: (): Map<number, BalanceResponse> => {
    const parsed = parseJson<Record<string, unknown>>(localStorage.getItem(BALANCE_CACHE_STORAGE_KEY))
    if (!parsed) {
      return new Map()
    }

    const entries = Object.entries(parsed)
      .map(([key, value]) => {
        const id = Number(key)
        if (!Number.isFinite(id) || !isBalanceResponse(value)) {
          return null
        }
        return [id, value] as const
      })
      .filter((entry): entry is readonly [number, BalanceResponse] => entry !== null)

    return new Map(entries)
  },

  setBalanceCache: (balances: Map<number, BalanceResponse>) => {
    if (balances.size === 0) {
      localStorage.removeItem(BALANCE_CACHE_STORAGE_KEY)
      return
    }

    const serialized = Object.fromEntries(balances.entries())
    localStorage.setItem(BALANCE_CACHE_STORAGE_KEY, JSON.stringify(serialized))
  },

  removeBalanceCache: () => localStorage.removeItem(BALANCE_CACHE_STORAGE_KEY),

  getCreditsSyncCursor: (): CreditsSyncCursor | null => {
    const parsed = parseJson<Partial<CreditsSyncCursor>>(localStorage.getItem(CREDITS_CURSOR_STORAGE_KEY))
    if (
      !parsed ||
      typeof parsed.lastTimestamp !== 'number' ||
      !Array.isArray(parsed.processedKeysAtTimestamp) ||
      parsed.processedKeysAtTimestamp.some(key => typeof key !== 'string')
    ) {
      return null
    }

    return {
      lastTimestamp: parsed.lastTimestamp,
      processedKeysAtTimestamp: parsed.processedKeysAtTimestamp,
    }
  },

  setCreditsSyncCursor: (cursor: CreditsSyncCursor) => {
    localStorage.setItem(CREDITS_CURSOR_STORAGE_KEY, JSON.stringify(cursor))
  },

  removeCreditsSyncCursor: () => localStorage.removeItem(CREDITS_CURSOR_STORAGE_KEY),
}
