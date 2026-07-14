import { useState, useEffect, useCallback } from 'react'
import { RefreshCw, ArrowLeft } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { getRequestLogs, getRequestStats, type RequestRecord, type RequestStats } from '@/api/requests'

interface RequestLogPageProps {
  onBack: () => void
}

export function RequestLogPage({ onBack }: RequestLogPageProps) {
  const [records, setRecords] = useState<RequestRecord[]>([])
  const [stats, setStats] = useState<RequestStats | null>(null)
  const [total, setTotal] = useState(0)
  const [page, setPage] = useState(1)
  const [loading, setLoading] = useState(false)
  const [callerFilter, setCallerFilter] = useState<string | null>(null)
  const pageSize = 50

  const fetchData = useCallback(async () => {
    setLoading(true)
    try {
      const [logRes, statsRes] = await Promise.all([
        getRequestLogs(page, pageSize, callerFilter ?? undefined),
        getRequestStats(),
      ])
      setRecords(logRes.records)
      setTotal(logRes.total)
      setStats(statsRes)
    } catch {
      // ignore
    } finally {
      setLoading(false)
    }
  }, [page, callerFilter])

  const handleCallerClick = (caller: string) => {
    setCallerFilter(prev => prev === caller ? null : caller)
    setPage(1)
  }

  useEffect(() => {
    fetchData()
  }, [fetchData])

  // 自动刷新（每 10 秒）
  useEffect(() => {
    const timer = setInterval(fetchData, 10000)
    return () => clearInterval(timer)
  }, [fetchData])

  const totalPages = Math.ceil(total / pageSize)

  return (
    <div className="min-h-screen bg-background">
      {/* 顶部导航 */}
      <header className="sticky top-0 z-50 w-full border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
        <div className="container flex h-14 items-center justify-between px-4 md:px-8">
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="icon" onClick={onBack}>
              <ArrowLeft className="h-5 w-5" />
            </Button>
            <span className="font-semibold">请求记录</span>
            <Badge variant="secondary">{total} 条</Badge>
            {callerFilter && (
              <Badge
                variant="default"
                className="cursor-pointer gap-1"
                onClick={() => { setCallerFilter(null); setPage(1) }}
              >
                {callerFilter} ✕
              </Badge>
            )}
          </div>
          <Button variant="ghost" size="icon" onClick={fetchData} disabled={loading}>
            <RefreshCw className={`h-5 w-5 ${loading ? 'animate-spin' : ''}`} />
          </Button>
        </div>
      </header>

      <main className="container mx-auto px-4 md:px-8 py-6">
        {/* 统计卡片 */}
        {stats && (
          <div className="grid gap-4 md:grid-cols-3 lg:grid-cols-8 mb-6">
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm font-medium text-muted-foreground">今日请求</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold">{stats.total}</div>
              </CardContent>
            </Card>
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm font-medium text-muted-foreground">成功率</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold text-green-600">
                  {stats.total > 0 ? ((stats.successCount / stats.total) * 100).toFixed(1) : 0}%
                </div>
              </CardContent>
            </Card>
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm font-medium text-muted-foreground">Credits</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold text-amber-600">{formatCredits(stats.totalCredits)}</div>
              </CardContent>
            </Card>
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm font-medium text-muted-foreground">输入 Tokens</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold">{(stats.totalInputTokens - stats.totalCacheReadTokens).toLocaleString()}</div>
              </CardContent>
            </Card>
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm font-medium text-muted-foreground">输出 Tokens</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold">{stats.totalOutputTokens.toLocaleString()}</div>
              </CardContent>
            </Card>
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm font-medium text-muted-foreground">模拟缓存 Tokens</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold text-green-500">{formatTokens(stats.totalCacheReadTokens)}</div>
              </CardContent>
            </Card>
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm font-medium text-muted-foreground">平均耗时</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold">{formatDuration(stats.avgDurationMs)}</div>
              </CardContent>
            </Card>
            <Card>
              <CardHeader className="pb-2">
                <CardTitle className="text-sm font-medium text-muted-foreground">平均首字</CardTitle>
              </CardHeader>
              <CardContent>
                <div className="text-2xl font-bold">{formatDuration(stats.avgTtftMs)}</div>
              </CardContent>
            </Card>
          </div>
        )}

        {/* 请求记录表格 */}
        <Card>
          <CardContent className="p-0">
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b bg-muted/50">
                    <th className="text-left p-3 font-medium">时间</th>
                    <th className="text-left p-3 font-medium">模型</th>
                    <th className="text-right p-3 font-medium">输入</th>
                    <th className="text-right p-3 font-medium">输出</th>
                    <th className="text-right p-3 font-medium">模拟缓存</th>
                    <th className="text-right p-3 font-medium">Credits</th>
                    <th className="text-right p-3 font-medium">首字</th>
                    <th className="text-right p-3 font-medium">总耗时</th>
                    <th className="text-center p-3 font-medium">类型</th>
                    <th className="text-center p-3 font-medium">调用者</th>
                    <th className="text-center p-3 font-medium">凭据</th>
                    <th className="text-center p-3 font-medium">状态</th>
                  </tr>
                </thead>
                <tbody>
                  {records.map((record, idx) => (
                    <tr key={idx} className="border-b hover:bg-muted/30">
                      <td className="p-3 whitespace-nowrap text-muted-foreground">
                        {formatTime(record.timestamp)}
                      </td>
                      <td className="p-3 whitespace-nowrap font-mono text-xs">
                        {shortenModel(record.model)}
                        {record.thinkingEffort && (
                          <Badge variant="outline" className="ml-1 text-xs">
                            {record.thinkingEffort}
                          </Badge>
                        )}
                      </td>
                      <td className="p-3 text-right tabular-nums">{Math.max(0, record.inputTokens - record.cacheReadTokens).toLocaleString()}</td>
                      <td className="p-3 text-right tabular-nums">{record.outputTokens.toLocaleString()}</td>
                      <td className="p-3 text-right tabular-nums text-green-500">{record.cacheReadTokens > 0 ? formatTokenCount(record.cacheReadTokens) : '-'}</td>
                      <td className="p-3 text-right tabular-nums text-amber-600">
                        {record.credits > 0 ? formatCredits(record.credits) : '-'}
                      </td>
                      <td className="p-3 text-right tabular-nums text-muted-foreground">
                        {record.ttftMs != null ? formatDuration(record.ttftMs) : '-'}
                      </td>
                      <td className="p-3 text-right tabular-nums">{formatDuration(record.durationMs)}</td>
                      <td className="p-3 text-center">
                        <Badge variant="outline" className="text-xs">
                          {record.stream ? 'stream' : 'sync'}
                        </Badge>
                      </td>
                      <td className="p-3 text-center">
                        {record.caller ? (
                          <Badge
                            variant={callerFilter === record.caller ? 'default' : 'secondary'}
                            className="text-xs cursor-pointer select-none"
                            onClick={() => handleCallerClick(record.caller!)}
                          >
                            {record.caller}
                          </Badge>
                        ) : (
                          <span className="text-muted-foreground">-</span>
                        )}
                      </td>
                      <td className="p-3 text-center text-muted-foreground">
                        {record.credentialId != null ? `#${record.credentialId}` : '-'}
                      </td>
                      <td className="p-3 text-center">
                        {record.success ? (
                          <Badge variant="success" className="text-xs">OK</Badge>
                        ) : (
                          <Badge variant="destructive" className="text-xs">ERR</Badge>
                        )}
                      </td>
                    </tr>
                  ))}
                  {records.length === 0 && (
                    <tr>
                      <td colSpan={12} className="p-8 text-center text-muted-foreground">
                        暂无请求记录
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </CardContent>
        </Card>

        {/* 分页 */}
        {totalPages > 1 && (
          <div className="flex justify-center items-center gap-4 mt-6">
            <Button
              variant="outline"
              size="sm"
              onClick={() => setPage(p => Math.max(1, p - 1))}
              disabled={page === 1}
            >
              上一页
            </Button>
            <span className="text-sm text-muted-foreground">
              第 {page} / {totalPages} 页
            </span>
            <Button
              variant="outline"
              size="sm"
              onClick={() => setPage(p => Math.min(totalPages, p + 1))}
              disabled={page === totalPages}
            >
              下一页
            </Button>
          </div>
        )}
      </main>
    </div>
  )
}

function formatTime(ts: number): string {
  const d = new Date(ts)
  return d.toLocaleTimeString('zh-CN', { hour12: false })
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`
  return `${(ms / 1000).toFixed(1)}s`
}

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return String(n)
}

function formatTokenCount(n: number): string {
  if (n >= 10_000) return `${Math.round(n / 1_000)}k`
  return n.toLocaleString()
}

function formatCredits(n: number): string {
  if (n >= 1) return n.toFixed(2)
  if (n >= 0.01) return n.toFixed(3)
  if (n >= 0.001) return n.toFixed(4)
  return n.toFixed(6)
}

function shortenModel(model: string): string {
  return model
    .replace('claude-', '')
    .replace('-20251101', '')
    .replace('-20250929', '')
}