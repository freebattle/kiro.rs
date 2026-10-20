import { memo, useState } from 'react'
import { toast } from 'sonner'
import { Trash2, Loader2, RefreshCw, KeyRound, Flame, Power } from 'lucide-react'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Switch } from '@/components/ui/switch'
import { Input } from '@/components/ui/input'
import { Checkbox } from '@/components/ui/checkbox'
import { Progress } from '@/components/ui/progress'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import type { CredentialStatusItem, BalanceResponse } from '@/types/api'
import { useSetDisabled, useSetPriority, useDeleteCredential } from '@/hooks/use-credentials'
import { getCredentialHealth, HEALTH_STYLES } from '@/lib/credential-health'
import { cn } from '@/lib/utils'

interface CredentialCardProps {
  credential: CredentialStatusItem
  /** 是否处于集中消耗（优先级唯一最高） */
  focused: boolean
  selected: boolean
  onToggleSelect: (id: number) => void
  balance: BalanceResponse | null
  loadingBalance: boolean
  onQueryBalance: (id: number) => void
  onRelogin: (credential: CredentialStatusItem) => void
  onFocus: (id: number) => void
  onUnfocus: () => void
}

function formatRelative(iso: string | null | undefined): string {
  if (!iso) return '从未'
  const diff = Date.now() - new Date(iso).getTime()
  if (diff < 60_000) return '刚刚'
  const minutes = Math.floor(diff / 60_000)
  if (minutes < 60) return `${minutes} 分钟前`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} 小时前`
  return `${Math.floor(hours / 24)} 天前`
}

function authLabel(c: CredentialStatusItem): string {
  if (c.authMethod === 'api_key') return 'API Key'
  if (c.authMethod === 'external_idp') return '企业 SSO'
  if (c.authMethod === 'social') return 'Social'
  if (c.authMethod === 'idc') return c.provider === 'BuilderId' ? 'Builder ID' : '企业 IdC'
  return c.authMethod ?? '未知'
}

// memo：Dashboard 的进度/选择等状态变化不再连带重渲染整页卡片
export const CredentialCard = memo(function CredentialCard({
  credential,
  focused,
  selected,
  onToggleSelect,
  balance,
  loadingBalance,
  onQueryBalance,
  onRelogin,
  onFocus,
  onUnfocus,
}: CredentialCardProps) {
  const [editingPriority, setEditingPriority] = useState(false)
  const [priorityValue, setPriorityValue] = useState(String(credential.priority))
  const [showDeleteDialog, setShowDeleteDialog] = useState(false)

  const setDisabled = useSetDisabled()
  const setPriority = useSetPriority()
  const deleteCredential = useDeleteCredential()

  const health = getCredentialHealth(credential)
  const style = HEALTH_STYLES[health.level]
  const canRelogin = credential.authMethod !== 'api_key'
  const subscription = balance?.subscriptionTitle || credential.subscriptionTitle

  // 三个操作都做了乐观更新，界面先变、请求后台完成；用 mutateAsync 保证卡片
  // 被乐观移除或翻页卸载后 toast 仍能弹出（mutate 的回调在组件卸载后不会触发）
  const toggleDisabled = async (disabled: boolean) => {
    if (setDisabled.isPending) return
    try {
      const res = await setDisabled.mutateAsync({ id: credential.id, disabled })
      toast.success(res.message)
    } catch (err) {
      toast.error('操作失败: ' + (err as Error).message)
    }
  }

  const startEditPriority = () => {
    setPriorityValue(String(credential.priority))
    setEditingPriority(true)
  }

  const handlePriorityChange = async () => {
    const newPriority = parseInt(priorityValue, 10)
    if (isNaN(newPriority) || newPriority < 0) {
      toast.error('优先级必须是非负整数')
      return
    }
    setEditingPriority(false)
    if (newPriority === credential.priority) return
    try {
      const res = await setPriority.mutateAsync({ id: credential.id, priority: newPriority })
      toast.success(res.message)
    } catch (err) {
      toast.error('操作失败: ' + (err as Error).message)
    }
  }

  const handleDelete = async () => {
    setShowDeleteDialog(false)
    if (!credential.disabled) {
      toast.error('请先停用凭据再删除')
      return
    }
    try {
      const res = await deleteCredential.mutateAsync(credential.id)
      toast.success(res.message)
    } catch (err) {
      toast.error('删除失败: ' + (err as Error).message)
    }
  }

  const remainingPercent = balance && balance.usageLimit > 0
    ? Math.max(0, 100 - balance.usagePercentage)
    : null

  return (
    <>
      <Card
        className={cn(
          'flex flex-col',
          focused && 'ring-2 ring-orange-400',
          health.level === 'relogin' && 'border-red-300 dark:border-red-900',
        )}
      >
        <CardContent className="p-3 flex flex-1 flex-col gap-2.5">
          {/* 标题：账号 + 启用开关 */}
          <div className="flex items-start justify-between gap-2">
            <div className="flex items-start gap-2 min-w-0">
              <Checkbox
                className="mt-0.5"
                checked={selected}
                onCheckedChange={() => onToggleSelect(credential.id)}
                aria-label={`选择凭据 #${credential.id}`}
              />
              <div className="min-w-0">
                <div className="text-sm font-medium truncate" title={credential.email || undefined}>
                  {credential.email || `凭据 #${credential.id}`}
                </div>
                <div className="text-xs text-muted-foreground truncate">
                  #{credential.id} · {authLabel(credential)}
                  {subscription ? ` · ${subscription}` : ''}
                </div>
              </div>
            </div>
            <Switch
              checked={!credential.disabled}
              onCheckedChange={(checked) => toggleDisabled(!checked)}
              title={credential.disabled ? '启用凭据' : '停用凭据'}
              aria-label={credential.disabled ? '启用凭据' : '停用凭据'}
            />
          </div>

          {/* 状态 + 优先级 */}
          <div className="flex items-center justify-between gap-2 text-xs">
            <span className={cn('inline-flex items-center gap-1.5 font-medium', style.text)}>
              <span className={cn('h-2 w-2 rounded-full', style.dot)} />
              {health.label}
            </span>
            <span className="inline-flex items-center gap-1.5">
              {focused && (
                <Badge className="h-5 px-1.5 border-transparent bg-orange-500 text-white hover:bg-orange-500/80">
                  集中消耗
                </Badge>
              )}
              {editingPriority ? (
                <span className="inline-flex items-center gap-1">
                  <Input
                    type="number"
                    value={priorityValue}
                    onChange={(e) => setPriorityValue(e.target.value)}
                    onKeyDown={(e) => {
                      if (e.key === 'Enter') handlePriorityChange()
                      if (e.key === 'Escape') setEditingPriority(false)
                    }}
                    onBlur={handlePriorityChange}
                    className="w-14 h-6 text-xs"
                    min="0"
                    autoFocus
                    aria-label="优先级"
                  />
                </span>
              ) : (
                <button
                  type="button"
                  className="text-muted-foreground hover:text-foreground hover:underline tabular-nums"
                  title="优先级：数字越小越优先，点击修改"
                  onClick={startEditPriority}
                >
                  优先级 {credential.priority}
                </button>
              )}
            </span>
          </div>

          {/* 登录态问题：原因 + 最近错误 + 推荐操作 */}
          {health.level !== 'ok' && health.level !== 'off' && (
            <div className={cn('rounded-md border p-2 text-xs space-y-1.5', style.panel)}>
              {health.hint && <p>{health.hint}</p>}
              {credential.lastError && (
                <p
                  className="font-mono text-[11px] text-muted-foreground line-clamp-2 break-all"
                  title={credential.lastError}
                >
                  {credential.lastError}
                  {credential.lastErrorAt ? `（${formatRelative(credential.lastErrorAt)}）` : ''}
                </p>
              )}
              {(health.action === 'relogin' || health.action === 'enable') && (
                <div className="flex gap-1.5 pt-0.5">
                  {canRelogin && (
                    <Button
                      size="sm"
                      className="h-6 px-2 text-xs"
                      variant={health.action === 'relogin' ? 'default' : 'outline'}
                      onClick={() => onRelogin(credential)}
                    >
                      <KeyRound className="h-3 w-3 mr-1" />
                      重新登录
                    </Button>
                  )}
                  {credential.disabled && (
                    <Button
                      size="sm"
                      className="h-6 px-2 text-xs"
                      variant={health.action === 'enable' ? 'default' : 'outline'}
                      onClick={() => toggleDisabled(false)}
                    >
                      <Power className="h-3 w-3 mr-1" />
                      启用重试
                    </Button>
                  )}
                </div>
              )}
            </div>
          )}

          {/* 剩余额度 */}
          <div className="space-y-1">
            <div className="flex items-center justify-between gap-2 text-xs">
              <span className="text-muted-foreground">剩余额度</span>
              <span className="inline-flex items-center gap-1">
                {balance ? (
                  <span className="font-medium tabular-nums">
                    {balance.remaining.toFixed(1)}
                    <span className="text-muted-foreground"> / {balance.usageLimit.toFixed(0)}</span>
                    {remainingPercent !== null && (
                      <span className="text-muted-foreground ml-1">({remainingPercent.toFixed(0)}%)</span>
                    )}
                  </span>
                ) : (
                  <span className="text-muted-foreground">未查询</span>
                )}
                <button
                  type="button"
                  className="p-0.5 rounded text-muted-foreground hover:text-foreground disabled:opacity-50"
                  onClick={() => onQueryBalance(credential.id)}
                  disabled={loadingBalance}
                  title="查询余额"
                  aria-label="查询余额"
                >
                  {loadingBalance ? (
                    <Loader2 className="h-3 w-3 animate-spin" />
                  ) : (
                    <RefreshCw className="h-3 w-3" />
                  )}
                </button>
              </span>
            </div>
            {balance && balance.usageLimit > 0 && (
              <Progress value={balance.usagePercentage} className="h-1.5" />
            )}
          </div>

          <div className="text-xs text-muted-foreground">
            成功 {credential.successCount} 次 · 最后调用 {formatRelative(credential.lastUsedAt)}
          </div>

          {/* 常用操作 */}
          <div className="mt-auto flex items-center justify-between gap-1 pt-2 border-t">
            <div className="flex items-center gap-1">
              {focused ? (
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-6 px-2 text-xs text-orange-600"
                  onClick={onUnfocus}
                  title="所有凭据优先级归 0，恢复均衡使用"
                >
                  <Flame className="h-3 w-3 mr-1" />
                  取消集中
                </Button>
              ) : (
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-6 px-2 text-xs"
                  onClick={() => onFocus(credential.id)}
                  disabled={credential.disabled}
                  title="把优先级设为唯一最高，请求只走这个凭据，直到它停用"
                >
                  <Flame className="h-3 w-3 mr-1" />
                  集中消耗
                </Button>
              )}
              {canRelogin && health.action !== 'relogin' && (
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-6 px-2 text-xs"
                  onClick={() => onRelogin(credential)}
                  title="通过浏览器重新授权，原地更新该凭据的 Token"
                >
                  <KeyRound className="h-3 w-3 mr-1" />
                  重新登录
                </Button>
              )}
            </div>
            {credential.disabled && (
              <Button
                size="sm"
                variant="ghost"
                className="h-6 w-6 p-0 text-destructive hover:text-destructive"
                onClick={() => setShowDeleteDialog(true)}
                title="删除凭据"
                aria-label="删除凭据"
              >
                <Trash2 className="h-3 w-3" />
              </Button>
            )}
          </div>
        </CardContent>
      </Card>

      {/* 删除确认对话框 */}
      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>确认删除凭据</DialogTitle>
            <DialogDescription>
              确定要删除凭据 #{credential.id}
              {credential.email ? `（${credential.email}）` : ''} 吗？此操作无法撤销。
              登录态失效时可以直接「重新登录」原地修复，不需要删除。
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button variant="outline" onClick={() => setShowDeleteDialog(false)}>
              取消
            </Button>
            <Button variant="destructive" onClick={handleDelete}>
              确认删除
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
})
