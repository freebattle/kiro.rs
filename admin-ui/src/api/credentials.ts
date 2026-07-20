import axios from 'axios'
import { storage } from '@/lib/storage'
import type {
  CredentialsStatusResponse,
  BalanceResponse,
  SuccessResponse,
  SetDisabledRequest,
  SetPriorityRequest,
  AddCredentialRequest,
  AddCredentialResponse,
  StartIdcLoginRequest,
  StartIdcLoginResponse,
  PollIdcLoginResponse,
  StartSocialLoginRequest,
  StartSocialLoginResponse,
  PollSocialLoginResponse,
  CompleteSocialLoginRequest,
} from '@/types/api'

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

// 获取所有凭据状态
export async function getCredentials(): Promise<CredentialsStatusResponse> {
  const { data } = await api.get<CredentialsStatusResponse>('/credentials')
  return data
}

// 设置凭据禁用状态
export async function setCredentialDisabled(
  id: number,
  disabled: boolean
): Promise<SuccessResponse> {
  const { data } = await api.post<SuccessResponse>(
    `/credentials/${id}/disabled`,
    { disabled } as SetDisabledRequest
  )
  return data
}

// 设置凭据优先级
export async function setCredentialPriority(
  id: number,
  priority: number
): Promise<SuccessResponse> {
  const { data } = await api.post<SuccessResponse>(
    `/credentials/${id}/priority`,
    { priority } as SetPriorityRequest
  )
  return data
}

// 重置失败计数
export async function resetCredentialFailure(
  id: number
): Promise<SuccessResponse> {
  const { data } = await api.post<SuccessResponse>(`/credentials/${id}/reset`)
  return data
}

// 强制刷新 Token
export async function forceRefreshToken(
  id: number
): Promise<SuccessResponse> {
  const { data } = await api.post<SuccessResponse>(`/credentials/${id}/refresh`)
  return data
}

// 获取凭据余额
export async function getCredentialBalance(id: number): Promise<BalanceResponse> {
  const { data } = await api.get<BalanceResponse>(`/credentials/${id}/balance`)
  return data
}

// 添加新凭据
export async function addCredential(
  req: AddCredentialRequest
): Promise<AddCredentialResponse> {
  const { data } = await api.post<AddCredentialResponse>('/credentials', req)
  return data
}

// 删除凭据
export async function deleteCredential(id: number): Promise<SuccessResponse> {
  const { data } = await api.delete<SuccessResponse>(`/credentials/${id}`)
  return data
}

// 获取负载均衡模式
export async function getLoadBalancingMode(): Promise<{ mode: 'priority' | 'balanced' }> {
  const { data } = await api.get<{ mode: 'priority' | 'balanced' }>('/config/load-balancing')
  return data
}

// ============ OAuth 登录授权 ============

export async function startIdcLogin(
  req: StartIdcLoginRequest
): Promise<StartIdcLoginResponse> {
  const { data } = await api.post<StartIdcLoginResponse>('/auth/idc/start', req)
  return data
}

export async function pollIdcLogin(sessionId: string): Promise<PollIdcLoginResponse> {
  const { data } = await api.post<PollIdcLoginResponse>(`/auth/idc/poll/${sessionId}`)
  return data
}

export async function startSocialLogin(
  req: StartSocialLoginRequest
): Promise<StartSocialLoginResponse> {
  const { data } = await api.post<StartSocialLoginResponse>('/auth/social/start', req)
  return data
}

export async function pollSocialLogin(sessionId: string): Promise<PollSocialLoginResponse> {
  const { data } = await api.post<PollSocialLoginResponse>(`/auth/social/poll/${sessionId}`)
  return data
}

export async function completeSocialLogin(
  sessionId: string,
  req: CompleteSocialLoginRequest
): Promise<PollSocialLoginResponse> {
  const { data } = await api.post<PollSocialLoginResponse>(
    `/auth/social/complete/${sessionId}`,
    req
  )
  return data
}

export async function startSocialRelogin(
  credentialId: number,
  req: StartSocialLoginRequest
): Promise<StartSocialLoginResponse> {
  const { data } = await api.post<StartSocialLoginResponse>(
    `/credentials/${credentialId}/relogin/social/start`,
    req
  )
  return data
}

export async function pollSocialRelogin(
  credentialId: number,
  sessionId: string
): Promise<PollSocialLoginResponse> {
  const { data } = await api.post<PollSocialLoginResponse>(
    `/credentials/${credentialId}/relogin/social/poll/${sessionId}`
  )
  return data
}

export async function completeSocialRelogin(
  credentialId: number,
  sessionId: string,
  req: CompleteSocialLoginRequest
): Promise<PollSocialLoginResponse> {
  const { data } = await api.post<PollSocialLoginResponse>(
    `/credentials/${credentialId}/relogin/social/complete/${sessionId}`,
    req
  )
  return data
}

export async function startIdcRelogin(
  credentialId: number,
  req: StartIdcLoginRequest
): Promise<StartIdcLoginResponse> {
  const { data } = await api.post<StartIdcLoginResponse>(
    `/credentials/${credentialId}/relogin/idc/start`,
    req
  )
  return data
}

export async function pollIdcRelogin(
  credentialId: number,
  sessionId: string
): Promise<PollIdcLoginResponse> {
  const { data } = await api.post<PollIdcLoginResponse>(
    `/credentials/${credentialId}/relogin/idc/poll/${sessionId}`
  )
  return data
}

// 设置负载均衡模式
export async function setLoadBalancingMode(mode: 'priority' | 'balanced'): Promise<{ mode: 'priority' | 'balanced' }> {
  const { data } = await api.put<{ mode: 'priority' | 'balanced' }>('/config/load-balancing', { mode })
  return data
}
