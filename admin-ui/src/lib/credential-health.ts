import type { CredentialStatusItem } from '@/types/api'

/** 与后端 MAX_FAILURES_PER_CREDENTIAL 保持一致 */
export const MAX_FAILURES = 3

export type HealthLevel = 'ok' | 'warn' | 'relogin' | 'quota' | 'off' | 'error'

export interface CredentialHealth {
  level: HealthLevel
  label: string
  /** 给用户的一句话说明和建议操作 */
  hint?: string
  /** 需要人工处理（重新登录 / 排查） */
  needsAttention: boolean
  /** 推荐的主操作 */
  action?: 'relogin' | 'enable'
}

/** 额度用尽后下一次重置时间：每月 1 日 00:30 UTC（与后端 next_monthly_quota_reset 一致） */
export function nextQuotaReset(from: Date): Date {
  const reset = new Date(Date.UTC(from.getUTCFullYear(), from.getUTCMonth(), 1, 0, 30))
  if (reset.getTime() <= from.getTime()) {
    reset.setUTCMonth(reset.getUTCMonth() + 1)
  }
  return reset
}

function formatDate(d: Date): string {
  return d.toLocaleString(undefined, { month: 'numeric', day: 'numeric', hour: '2-digit', minute: '2-digit' })
}

export function getCredentialHealth(c: CredentialStatusItem): CredentialHealth {
  if (c.disabled) {
    switch (c.disabledReason) {
      case 'InvalidRefreshToken':
        return {
          level: 'relogin',
          label: '登录已失效',
          hint: 'refreshToken 已被服务端吊销或过期，需要重新登录',
          needsAttention: true,
          action: 'relogin',
        }
      case 'TooManyRefreshFailures':
        return {
          level: 'relogin',
          label: 'Token 刷新失败',
          hint: `连续 ${MAX_FAILURES} 次刷新失败已自动停用；网络问题可直接启用重试，否则重新登录`,
          needsAttention: true,
          action: 'relogin',
        }
      case 'TooManyFailures':
        return {
          level: 'relogin',
          label: '鉴权失败',
          hint: `连续 ${MAX_FAILURES} 次 401/403 已自动停用；可先启用重试，仍失败再重新登录`,
          needsAttention: true,
          action: 'enable',
        }
      case 'QuotaExceeded': {
        const since = c.quotaExhaustedAt ? new Date(c.quotaExhaustedAt) : new Date()
        return {
          level: 'quota',
          label: '额度用尽',
          hint: `预计 ${formatDate(nextQuotaReset(since))} 额度重置后自动恢复`,
          needsAttention: false,
        }
      }
      case 'InvalidConfig':
        return {
          level: 'error',
          label: '配置无效',
          hint: '凭据配置缺失必要字段，需修正 credentials.json 后重启',
          needsAttention: true,
        }
      default:
        return { level: 'off', label: '已停用', hint: '手动停用', needsAttention: false }
    }
  }

  const failures = Math.max(c.failureCount, c.refreshFailureCount)
  if (failures > 0) {
    return {
      level: 'warn',
      label: `异常 ${failures}/${MAX_FAILURES}`,
      hint:
        c.refreshFailureCount > 0
          ? `Token 刷新失败 ${c.refreshFailureCount} 次，达到 ${MAX_FAILURES} 次会自动停用`
          : `请求鉴权失败 ${c.failureCount} 次，达到 ${MAX_FAILURES} 次会自动停用`,
      needsAttention: true,
      action: c.refreshFailureCount > 0 ? 'relogin' : undefined,
    }
  }

  return { level: 'ok', label: '正常', needsAttention: false }
}

export const HEALTH_STYLES: Record<HealthLevel, { dot: string; text: string; panel: string }> = {
  ok: { dot: 'bg-green-500', text: 'text-green-600 dark:text-green-400', panel: '' },
  warn: {
    dot: 'bg-amber-500',
    text: 'text-amber-600 dark:text-amber-400',
    panel: 'border-amber-300 bg-amber-50 dark:border-amber-900 dark:bg-amber-950/40',
  },
  relogin: {
    dot: 'bg-red-500',
    text: 'text-red-600 dark:text-red-400',
    panel: 'border-red-300 bg-red-50 dark:border-red-900 dark:bg-red-950/40',
  },
  error: {
    dot: 'bg-red-500',
    text: 'text-red-600 dark:text-red-400',
    panel: 'border-red-300 bg-red-50 dark:border-red-900 dark:bg-red-950/40',
  },
  quota: {
    dot: 'bg-slate-400',
    text: 'text-slate-600 dark:text-slate-300',
    panel: 'border-slate-300 bg-slate-50 dark:border-slate-700 dark:bg-slate-900/40',
  },
  off: { dot: 'bg-slate-300', text: 'text-muted-foreground', panel: '' },
}

/**
 * 是否处于「集中消耗」：该凭据启用，且优先级严格高于（数字小于）其他所有启用凭据。
 * 两种负载均衡模式下都会只用它，直到它被停用。
 */
export function isFocused(c: CredentialStatusItem, all: CredentialStatusItem[]): boolean {
  if (c.disabled) return false
  const others = all.filter(o => o.id !== c.id && !o.disabled)
  return others.length > 0 && others.every(o => o.priority > c.priority)
}

/** 集中消耗某个凭据：它设为 0，其余优先级为 0 的设为 1（其余保持不变） */
export function buildFocusUpdates(id: number, all: CredentialStatusItem[]) {
  return all
    .map(c => {
      if (c.id === id) return c.priority === 0 ? null : { id: c.id, priority: 0 }
      return c.priority === 0 ? { id: c.id, priority: 1 } : null
    })
    .filter((u): u is { id: number; priority: number } => u !== null)
}

/** 恢复均衡：所有凭据优先级归 0 */
export function buildResetUpdates(all: CredentialStatusItem[]) {
  return all.filter(c => c.priority !== 0).map(c => ({ id: c.id, priority: 0 }))
}
