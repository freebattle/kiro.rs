import axios from 'axios'
import { storage } from '@/lib/storage'

// 创建 axios 实例
const api = axios.create({
  baseURL: '/api/admin',
  headers: {
    'Content-Type': 'application/json',
  },
})

// 请求拦截器添加 API Key
api.interceptors.request.use((config) => {
  const apiKey = storage.getApiKey()
  if (apiKey) {
    config.headers['x-api-key'] = apiKey
  }
  return config
})

export interface RequestRecord {
  model: string
  inputTokens: number
  outputTokens: number
  ttftMs: number | null
  durationMs: number
  timestamp: number
  stream: boolean
  credentialId: number | null
  success: boolean
  credits: number
  caller?: string
}

export interface RequestLogResponse {
  records: RequestRecord[]
  total: number
  page: number
  pageSize: number
}

export interface RequestStats {
  total: number
  successCount: number
  totalInputTokens: number
  totalOutputTokens: number
  avgDurationMs: number
  avgTtftMs: number
  totalCredits: number
}

export interface ModelUsage {
  requests: number
  input_tokens: number
  output_tokens: number
  credits: number
}

export interface MonthlyUsage {
  month: string
  credentials: Record<string, Record<string, ModelUsage>>
  callers?: Record<string, Record<string, ModelUsage>>
}

export async function getRequestLogs(page = 1, pageSize = 50): Promise<RequestLogResponse> {
  const { data } = await api.get<RequestLogResponse>('/requests', {
    params: { page, pageSize },
  })
  return data
}

export async function getRequestStats(): Promise<RequestStats> {
  const { data } = await api.get<RequestStats>('/requests/stats')
  return data
}

export async function getUsageStats(month?: string): Promise<MonthlyUsage> {
  const { data } = await api.get<MonthlyUsage>('/usage-stats', {
    params: month ? { month } : undefined,
  })
  return data
}
