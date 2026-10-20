import { useQuery, useMutation, useQueryClient, type QueryClient } from '@tanstack/react-query'
import {
  getCredentials,
  setCredentialDisabled,
  setCredentialPriority,
  setCredentialPriorities,
  resetCredentialFailure,
  forceRefreshToken,
  getCredentialBalance,
  addCredential,
  deleteCredential,
  getLoadBalancingMode,
  setLoadBalancingMode,
} from '@/api/credentials'
import type { AddCredentialRequest, CredentialsStatusResponse } from '@/types/api'

const CREDENTIALS_KEY = ['credentials']

type OptimisticContext = { previous?: CredentialsStatusResponse }

// 乐观更新：先改本地缓存让界面立即响应，失败时回滚，结束后以服务端数据为准
async function applyOptimistic(
  queryClient: QueryClient,
  update: (data: CredentialsStatusResponse) => CredentialsStatusResponse,
): Promise<OptimisticContext> {
  await queryClient.cancelQueries({ queryKey: CREDENTIALS_KEY })
  const previous = queryClient.getQueryData<CredentialsStatusResponse>(CREDENTIALS_KEY)
  if (previous) {
    queryClient.setQueryData<CredentialsStatusResponse>(CREDENTIALS_KEY, update(previous))
  }
  return { previous }
}

function rollback(queryClient: QueryClient, context: OptimisticContext | undefined) {
  if (context?.previous) {
    queryClient.setQueryData(CREDENTIALS_KEY, context.previous)
  }
}

function countAvailable(credentials: CredentialsStatusResponse['credentials']): number {
  return credentials.filter(c => !c.disabled).length
}

// 查询凭据列表
export function useCredentials() {
  return useQuery({
    queryKey: ['credentials'],
    queryFn: getCredentials,
    refetchInterval: 30000, // 每 30 秒刷新一次
  })
}

// 查询凭据余额
export function useCredentialBalance(id: number | null) {
  return useQuery({
    queryKey: ['credential-balance', id],
    queryFn: () => getCredentialBalance(id!),
    enabled: id !== null,
    retry: false, // 余额查询失败时不重试（避免重复请求被封禁的账号）
  })
}

// 设置禁用状态
export function useSetDisabled() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, disabled }: { id: number; disabled: boolean }) =>
      setCredentialDisabled(id, disabled),
    onMutate: ({ id, disabled }) =>
      applyOptimistic(queryClient, data => {
        // 与后端 set_disabled 一致：启用会清空失败计数和禁用原因，手动禁用记为 Manual
        const credentials = data.credentials.map(c => {
          if (c.id !== id) return c
          return disabled
            ? { ...c, disabled, disabledReason: 'Manual' as const }
            : {
                ...c,
                disabled,
                disabledReason: undefined,
                failureCount: 0,
                refreshFailureCount: 0,
                lastError: undefined,
                lastErrorAt: undefined,
              }
        })
        return { ...data, credentials, available: countAvailable(credentials) }
      }),
    onError: (_err, _vars, context) => rollback(queryClient, context),
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: CREDENTIALS_KEY })
    },
  })
}

// 设置优先级
export function useSetPriority() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: ({ id, priority }: { id: number; priority: number }) =>
      setCredentialPriority(id, priority),
    onMutate: ({ id, priority }) =>
      applyOptimistic(queryClient, data => ({
        ...data,
        credentials: data.credentials.map(c => (c.id === id ? { ...c, priority } : c)),
      })),
    onError: (_err, _vars, context) => rollback(queryClient, context),
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: CREDENTIALS_KEY })
    },
  })
}

// 批量设置优先级（集中消耗 / 恢复均衡 / 置顶）
export function useSetPriorities() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (updates: { id: number; priority: number }[]) => setCredentialPriorities(updates),
    onMutate: updates =>
      applyOptimistic(queryClient, data => {
        const next = new Map(updates.map(u => [u.id, u.priority]))
        const credentials = data.credentials
          .map(c => (next.has(c.id) ? { ...c, priority: next.get(c.id)! } : c))
          .sort((a, b) => a.priority - b.priority || a.id - b.id)
        return { ...data, credentials }
      }),
    onError: (_err, _vars, context) => rollback(queryClient, context),
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: CREDENTIALS_KEY })
    },
  })
}

// 重置失败计数
export function useResetFailure() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: number) => resetCredentialFailure(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['credentials'] })
    },
  })
}

// 强制刷新 Token
export function useForceRefreshToken() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: number) => forceRefreshToken(id),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['credentials'] })
    },
  })
}

// 添加新凭据
export function useAddCredential() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (req: AddCredentialRequest) => addCredential(req),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['credentials'] })
    },
  })
}

// 删除凭据
export function useDeleteCredential() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: (id: number) => deleteCredential(id),
    onMutate: (id) =>
      applyOptimistic(queryClient, data => {
        const credentials = data.credentials.filter(c => c.id !== id)
        return {
          ...data,
          credentials,
          total: credentials.length,
          available: countAvailable(credentials),
        }
      }),
    onError: (_err, _id, context) => rollback(queryClient, context),
    onSettled: () => {
      queryClient.invalidateQueries({ queryKey: CREDENTIALS_KEY })
    },
  })
}

// 获取负载均衡模式
export function useLoadBalancingMode() {
  return useQuery({
    queryKey: ['loadBalancingMode'],
    queryFn: getLoadBalancingMode,
  })
}

// 设置负载均衡模式
export function useSetLoadBalancingMode() {
  const queryClient = useQueryClient()
  return useMutation({
    mutationFn: setLoadBalancingMode,
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['loadBalancingMode'] })
    },
  })
}
