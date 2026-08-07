import { useState } from 'react'
import { toast } from 'sonner'
import { Wallet, Trash2, Loader2 } from 'lucide-react'
import { Card, CardContent } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { Switch } from '@/components/ui/switch'
import { Input } from '@/components/ui/input'
import { Checkbox } from '@/components/ui/checkbox'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import type { CredentialStatusItem, BalanceResponse } from '@/types/api'
import {
  useSetDisabled,
  useSetPriority,
  useDeleteCredential,
} from '@/hooks/use-credentials'

interface CredentialCardProps {
  credential: CredentialStatusItem
  onQueryBalance: (id: number) => void
  selected: boolean
  onToggleSelect: () => void
  balance: BalanceResponse | null
  loadingBalance: boolean
}

function formatLastUsed(lastUsedAt: string | null): string {
  if (!lastUsedAt) return '从未使用'
  const date = new Date(lastUsedAt)
  const now = new Date()
  const diff = now.getTime() - date.getTime()
  if (diff < 0) return '刚刚'
  const seconds = Math.floor(diff / 1000)
  if (seconds < 60) return `${seconds} 秒前`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes} 分钟前`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours} 小时前`
  const days = Math.floor(hours / 24)
  return `${days} 天前`
}

function authMethodLabel(authMethod: string): string {
  switch (authMethod) {
    case 'api_key':
      return 'API Key'
    case 'idc':
      return 'IdC'
    case 'social':
      return 'Social'
    case 'external_idp':
      return '企业 SSO'
    default:
      return authMethod
  }
}

export function CredentialCard({
  credential,
  onQueryBalance,
  selected,
  onToggleSelect,
  balance,
  loadingBalance,
}: CredentialCardProps) {
  const [editingPriority, setEditingPriority] = useState(false)
  const [priorityValue, setPriorityValue] = useState(String(credential.priority))
  const [showDeleteDialog, setShowDeleteDialog] = useState(false)

  const setDisabled = useSetDisabled()
  const setPriority = useSetPriority()
  const deleteCredential = useDeleteCredential()

  const handleToggleDisabled = () => {
    setDisabled.mutate(
      { id: credential.id, disabled: !credential.disabled },
      {
        onSuccess: (res) => {
          toast.success(res.message)
        },
        onError: (err) => {
          toast.error('操作失败: ' + (err as Error).message)
        },
      }
    )
  }

  const handlePriorityChange = () => {
    const newPriority = parseInt(priorityValue, 10)
    if (isNaN(newPriority) || newPriority < 0) {
      toast.error('优先级必须是非负整数')
      return
    }
    setPriority.mutate(
      { id: credential.id, priority: newPriority },
      {
        onSuccess: (res) => {
          toast.success(res.message)
          setEditingPriority(false)
        },
        onError: (err) => {
          toast.error('操作失败: ' + (err as Error).message)
        },
      }
    )
  }

  const handleDelete = () => {
    if (!credential.disabled) {
      toast.error('请先禁用凭据再删除')
      setShowDeleteDialog(false)
      return
    }

    deleteCredential.mutate(credential.id, {
      onSuccess: (res) => {
        toast.success(res.message)
        setShowDeleteDialog(false)
      },
      onError: (err) => {
        toast.error('删除失败: ' + (err as Error).message)
      },
    })
  }

  return (
    <>
      <Card className={credential.isCurrent ? 'ring-2 ring-primary' : ''}>
        <CardContent className="p-3 space-y-2">
          {/* 标题行 */}
          <div className="flex items-center justify-between gap-2">
            <div className="flex items-center gap-2 min-w-0">
              <Checkbox checked={selected} onCheckedChange={onToggleSelect} />
              <span
                className="text-sm font-medium truncate"
                title={credential.email || `凭据 #${credential.id}`}
              >
                {credential.email || `凭据 #${credential.id}`}
              </span>
            </div>
            <Switch
              checked={!credential.disabled}
              onCheckedChange={handleToggleDisabled}
              disabled={setDisabled.isPending}
              title={credential.disabled ? '启用凭据' : '禁用凭据'}
            />
          </div>

          {/* 状态徽章 */}
          <div className="flex flex-wrap items-center gap-1">
            <Badge variant="outline">#{credential.id}</Badge>
            {credential.isCurrent && <Badge variant="success">当前</Badge>}
            {credential.disabled && (
              <Badge variant="destructive" title={credential.disabledReason || undefined}>
                已禁用
              </Badge>
            )}
            {credential.authMethod && (
              <Badge variant="secondary">{authMethodLabel(credential.authMethod)}</Badge>
            )}
          </div>

          {/* 关键信息 */}
          <div className="space-y-1 text-xs">
            <div className="flex items-center justify-between gap-2">
              <span className="text-muted-foreground">剩余用量</span>
              {loadingBalance ? (
                <Loader2 className="w-3 h-3 animate-spin" />
              ) : balance ? (
                <span className="font-medium tabular-nums">
                  {balance.remaining.toFixed(2)} / {balance.usageLimit.toFixed(2)}
                  <span className="text-muted-foreground ml-1">
                    ({(100 - balance.usagePercentage).toFixed(0)}%)
                  </span>
                </span>
              ) : (
                <span className="text-muted-foreground">未知</span>
              )}
            </div>
            <div className="flex items-center justify-between gap-2">
              <span className="text-muted-foreground">订阅</span>
              <span className="font-medium truncate">{balance?.subscriptionTitle || '未知'}</span>
            </div>
            <div className="flex items-center justify-between gap-2">
              <span className="text-muted-foreground">优先级</span>
              {editingPriority ? (
                <span className="inline-flex items-center gap-1">
                  <Input
                    type="number"
                    value={priorityValue}
                    onChange={(e) => setPriorityValue(e.target.value)}
                    className="w-14 h-6 text-xs"
                    min="0"
                  />
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-6 w-6 p-0"
                    onClick={handlePriorityChange}
                    disabled={setPriority.isPending}
                  >
                    ✓
                  </Button>
                  <Button
                    size="sm"
                    variant="ghost"
                    className="h-6 w-6 p-0"
                    onClick={() => {
                      setEditingPriority(false)
                      setPriorityValue(String(credential.priority))
                    }}
                  >
                    ✕
                  </Button>
                </span>
              ) : (
                <span
                  className="font-medium cursor-pointer hover:underline tabular-nums"
                  title="点击编辑优先级"
                  onClick={() => setEditingPriority(true)}
                >
                  {credential.priority}
                </span>
              )}
            </div>
            <div className="flex items-center justify-between gap-2">
              <span className="text-muted-foreground">成功 / 失败</span>
              <span
                className="font-medium tabular-nums"
                title={`刷新失败 ${credential.refreshFailureCount} 次`}
              >
                {credential.successCount}
                <span className="text-muted-foreground mx-1">/</span>
                <span className={credential.failureCount > 0 ? 'text-red-500' : ''}>
                  {credential.failureCount}
                </span>
              </span>
            </div>
            <div className="flex items-center justify-between gap-2">
              <span className="text-muted-foreground">最后调用</span>
              <span className="font-medium">{formatLastUsed(credential.lastUsedAt)}</span>
            </div>
          </div>

          {/* 操作按钮 */}
          <div className="flex gap-2 pt-2 border-t">
            <Button
              size="sm"
              variant="default"
              className="h-7 flex-1"
              onClick={() => onQueryBalance(credential.id)}
              disabled={loadingBalance}
            >
              {loadingBalance ? (
                <Loader2 className="h-3.5 w-3.5 mr-1 animate-spin" />
              ) : (
                <Wallet className="h-3.5 w-3.5 mr-1" />
              )}
              查询余额
            </Button>
            <Button
              size="sm"
              variant="destructive"
              className="h-7"
              onClick={() => setShowDeleteDialog(true)}
              disabled={!credential.disabled}
              title={!credential.disabled ? '需要先禁用凭据才能删除' : undefined}
            >
              <Trash2 className="h-3.5 w-3.5 mr-1" />
              删除
            </Button>
          </div>
        </CardContent>
      </Card>

      {/* 删除确认对话框 */}
      <Dialog open={showDeleteDialog} onOpenChange={setShowDeleteDialog}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>确认删除凭据</DialogTitle>
            <DialogDescription>
              您确定要删除凭据 #{credential.id} 吗？此操作无法撤销。
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              variant="outline"
              onClick={() => setShowDeleteDialog(false)}
              disabled={deleteCredential.isPending}
            >
              取消
            </Button>
            <Button
              variant="destructive"
              onClick={handleDelete}
              disabled={deleteCredential.isPending || !credential.disabled}
            >
              确认删除
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
