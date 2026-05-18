import { useState, useEffect, useCallback } from 'react'
import { ArrowLeft, ChevronLeft, ChevronRight } from 'lucide-react'
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card'
import { Button } from '@/components/ui/button'
import { Badge } from '@/components/ui/badge'
import { getUsageStats, type MonthlyUsage, type ModelUsage } from '@/api/requests'

interface UsageStatsPageProps {
  onBack: () => void
}

interface FlatRow {
  credentialId: string
  model: string
  usage: ModelUsage
}

interface CallerRow {
  caller: string
  model: string
  usage: ModelUsage
}

export function UsageStatsPage({ onBack }: UsageStatsPageProps) {
  const [month, setMonth] = useState(() => getCurrentMonth())
  const [data, setData] = useState<MonthlyUsage | null>(null)
  const [loading, setLoading] = useState(false)

  const fetchData = useCallback(async () => {
    setLoading(true)
    try {
      const res = await getUsageStats(month)
      setData(res)
    } catch {
      // ignore
    } finally {
      setLoading(false)
    }
  }, [month])

  useEffect(() => {
    fetchData()
  }, [fetchData])

  const rows = flattenUsage(data)
  const callerRows = flattenCallerUsage(data)
  const totals = computeTotals(rows)

  return (
    <div className="min-h-screen bg-background">
      <header className="sticky top-0 z-50 w-full border-b bg-background/95 backdrop-blur supports-[backdrop-filter]:bg-background/60">
        <div className="container flex h-14 items-center justify-between px-4 md:px-8">
          <div className="flex items-center gap-2">
            <Button variant="ghost" size="icon" onClick={onBack}>
              <ArrowLeft className="h-5 w-5" />
            </Button>
            <span className="font-semibold">用量统计</span>
          </div>
          <div className="flex items-center gap-1">
            <Button variant="ghost" size="icon" onClick={() => setMonth(prevMonth(month))}>
              <ChevronLeft className="h-4 w-4" />
            </Button>
            <span className="text-sm font-medium w-20 text-center">{month}</span>
            <Button variant="ghost" size="icon" onClick={() => setMonth(nextMonth(month))} disabled={month >= getCurrentMonth()}>
              <ChevronRight className="h-4 w-4" />
            </Button>
          </div>
        </div>
      </header>

      <main className="container mx-auto px-4 md:px-8 py-6">
        {/* 汇总卡片 */}
        <div className="grid gap-4 md:grid-cols-4 mb-6">
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm font-medium text-muted-foreground">总请求数</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold">{totals.requests.toLocaleString()}</div>
            </CardContent>
          </Card>
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm font-medium text-muted-foreground">总 Credits</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold text-amber-600">{formatCredits(totals.credits)}</div>
            </CardContent>
          </Card>
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm font-medium text-muted-foreground">总输入 Tokens</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold">{formatTokens(totals.inputTokens - totals.cacheReadTokens)}</div>
            </CardContent>
          </Card>
          <Card>
            <CardHeader className="pb-2">
              <CardTitle className="text-sm font-medium text-muted-foreground">总输出 Tokens</CardTitle>
            </CardHeader>
            <CardContent>
              <div className="text-2xl font-bold">{formatTokens(totals.outputTokens)}</div>
            </CardContent>
          </Card>
        </div>

        {/* 按调用者统计 */}
        {callerRows.length > 0 && (
          <Card className="mb-6">
            <CardHeader>
              <CardTitle className="text-base">按调用者统计</CardTitle>
            </CardHeader>
            <CardContent className="p-0">
              <div className="overflow-x-auto">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="border-b bg-muted/50">
                      <th className="text-left p-3 font-medium">调用者</th>
                      <th className="text-left p-3 font-medium">模型</th>
                      <th className="text-right p-3 font-medium">请求数</th>
                      <th className="text-right p-3 font-medium">Credits</th>
                      <th className="text-right p-3 font-medium">输入 Tokens</th>
                      <th className="text-right p-3 font-medium">输出 Tokens</th>
                    </tr>
                  </thead>
                  <tbody>
                    {callerRows.map((row, idx) => (
                      <tr key={idx} className="border-b hover:bg-muted/30">
                        <td className="p-3">
                          <Badge variant="secondary">{row.caller}</Badge>
                        </td>
                        <td className="p-3 font-mono text-xs">{shortenModel(row.model)}</td>
                        <td className="p-3 text-right tabular-nums">{row.usage.requests.toLocaleString()}</td>
                        <td className="p-3 text-right tabular-nums text-amber-600">{formatCredits(row.usage.credits)}</td>
                        <td className="p-3 text-right tabular-nums">{formatTokens(row.usage.input_tokens)}</td>
                        <td className="p-3 text-right tabular-nums">{formatTokens(row.usage.output_tokens)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </CardContent>
          </Card>
        )}

        {/* 按凭据明细表格 */}
        <Card>
          <CardContent className="p-0">
            <div className="overflow-x-auto">
              <table className="w-full text-sm">
                <thead>
                  <tr className="border-b bg-muted/50">
                    <th className="text-left p-3 font-medium">凭据</th>
                    <th className="text-left p-3 font-medium">模型</th>
                    <th className="text-right p-3 font-medium">请求数</th>
                    <th className="text-right p-3 font-medium">Credits</th>
                    <th className="text-right p-3 font-medium">输入 Tokens</th>
                    <th className="text-right p-3 font-medium">输出 Tokens</th>
                    <th className="text-right p-3 font-medium">模拟缓存</th>
                  </tr>
                </thead>
                <tbody>
                  {rows.map((row, idx) => (
                    <tr key={idx} className="border-b hover:bg-muted/30">
                      <td className="p-3">
                        <Badge variant="outline">#{row.credentialId}</Badge>
                      </td>
                      <td className="p-3 font-mono text-xs">{shortenModel(row.model)}</td>
                      <td className="p-3 text-right tabular-nums">{row.usage.requests.toLocaleString()}</td>
                      <td className="p-3 text-right tabular-nums text-amber-600">{formatCredits(row.usage.credits)}</td>
                      <td className="p-3 text-right tabular-nums">{formatTokens(row.usage.input_tokens)}</td>
                      <td className="p-3 text-right tabular-nums">{formatTokens(row.usage.output_tokens)}</td>
                      <td className="p-3 text-right tabular-nums">{row.usage.cache_read_tokens > 0 ? formatTokens(row.usage.cache_read_tokens) : '-'}</td>
                    </tr>
                  ))}
                  {rows.length === 0 && !loading && (
                    <tr>
                      <td colSpan={7} className="p-8 text-center text-muted-foreground">
                        本月暂无用量数据
                      </td>
                    </tr>
                  )}
                </tbody>
              </table>
            </div>
          </CardContent>
        </Card>
      </main>
    </div>
  )
}

function getCurrentMonth(): string {
  const now = new Date()
  return `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, '0')}`
}

function prevMonth(m: string): string {
  const [y, mo] = m.split('-').map(Number)
  if (mo === 1) return `${y - 1}-12`
  return `${y}-${String(mo - 1).padStart(2, '0')}`
}

function nextMonth(m: string): string {
  const [y, mo] = m.split('-').map(Number)
  if (mo === 12) return `${y + 1}-01`
  return `${y}-${String(mo + 1).padStart(2, '0')}`
}

function flattenUsage(data: MonthlyUsage | null): FlatRow[] {
  if (!data) return []
  const rows: FlatRow[] = []
  for (const [credId, models] of Object.entries(data.credentials)) {
    for (const [model, usage] of Object.entries(models)) {
      rows.push({ credentialId: credId, model, usage })
    }
  }
  rows.sort((a, b) => b.usage.requests - a.usage.requests)
  return rows
}

function flattenCallerUsage(data: MonthlyUsage | null): CallerRow[] {
  if (!data || !data.callers) return []
  const rows: CallerRow[] = []
  for (const [caller, models] of Object.entries(data.callers)) {
    for (const [model, usage] of Object.entries(models)) {
      rows.push({ caller, model, usage })
    }
  }
  rows.sort((a, b) => b.usage.requests - a.usage.requests)
  return rows
}

function computeTotals(rows: FlatRow[]) {
  let requests = 0, inputTokens = 0, outputTokens = 0, cacheReadTokens = 0, credits = 0
  for (const r of rows) {
    requests += r.usage.requests
    inputTokens += r.usage.input_tokens
    outputTokens += r.usage.output_tokens
    cacheReadTokens += r.usage.cache_read_tokens || 0
    credits += r.usage.credits || 0
  }
  return { requests, inputTokens, outputTokens, cacheReadTokens, credits }
}

function formatTokens(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return String(n)
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
