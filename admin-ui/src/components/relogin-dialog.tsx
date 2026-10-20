import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { CheckCircle, Copy, ExternalLink, Loader2 } from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import {
  completeSocialRelogin,
  pollIdcRelogin,
  pollSocialRelogin,
  startIdcRelogin,
  startSocialRelogin,
} from '@/api/credentials'
import type {
  CredentialStatusItem,
  StartIdcLoginResponse,
  StartSocialLoginResponse,
} from '@/types/api'
import { extractErrorMessage } from '@/lib/utils'

const BUILDER_ID_START_URL = 'https://view.awsapps.com/start'

interface ReloginDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  credential: CredentialStatusItem | null
  onSuccess: () => void
}

type Method = 'social' | 'idc'
type Step = 'select' | 'waiting' | 'done'

const POLL_INTERVAL_MS = 2000
const isRemoteAccess = () =>
  window.location.hostname !== 'localhost' && window.location.hostname !== '127.0.0.1'

function parseCallbackUrl(rawUrl: string): {
  code?: string
  state?: string
  loginOption?: string
  path: string
  issuerUrl?: string
  clientId?: string
  scopes?: string
  loginHint?: string
} | null {
  try {
    const url = new URL(rawUrl.trim())
    const hashParams = new URLSearchParams(url.hash.startsWith('#') ? url.hash.slice(1) : url.hash)
    const readParam = (...names: string[]) => {
      for (const name of names) {
        const value = url.searchParams.get(name) ?? hashParams.get(name)
        if (value) return value
      }
      return null
    }
    const code = readParam('code')
    const state = readParam('state')
    const issuerUrl = readParam('issuer_url', 'issuerUrl')
    const clientId = readParam('client_id', 'clientId')
    if ((!code || !state) && (!issuerUrl || !clientId)) return null
    return {
      code: code ?? undefined,
      state: state ?? undefined,
      loginOption: readParam('login_option', 'loginOption') ?? '',
      path: url.pathname,
      issuerUrl: issuerUrl ?? undefined,
      clientId: clientId ?? undefined,
      scopes: readParam('scopes', 'scope') ?? undefined,
      loginHint: readParam('login_hint', 'loginHint') ?? undefined,
    }
  } catch {
    return null
  }
}

function getContinueUrl(result: { status: string; nextUrl?: string; next_url?: string }): string {
  return result.status === 'continue' ? (result.nextUrl || result.next_url || '') : ''
}

export function ReloginDialog({
  open,
  onOpenChange,
  credential,
  onSuccess,
}: ReloginDialogProps) {
  const credentialId = credential?.id ?? 0
  const authMethod = credential?.authMethod ?? null
  const initialMethod: Method =
    authMethod === 'idc' || authMethod === 'builder-id' || authMethod === 'iam'
      ? 'idc'
      : 'social'
  // 沿用凭据原有的 Start URL / 区域：企业 IdC 凭据重新登录必须走同一个 IAM Identity Center
  const initialStartUrl = credential?.startUrl ?? ''
  const initialRegion = credential?.authRegion ?? 'us-east-1'
  const isEnterpriseIdc = !!initialStartUrl && initialStartUrl !== BUILDER_ID_START_URL

  const [method, setMethod] = useState<Method>(initialMethod)
  const [step, setStep] = useState<Step>('select')
  const [isStarting, setIsStarting] = useState(false)
  const [isCompleting, setIsCompleting] = useState(false)
  const [callbackUrl, setCallbackUrl] = useState('')
  const [nextUrl, setNextUrl] = useState('')
  const [socialSession, setSocialSession] = useState<StartSocialLoginResponse | null>(null)
  const [idcSession, setIdcSession] = useState<StartIdcLoginResponse | null>(null)
  const [region, setRegion] = useState(initialRegion)
  const [startUrl, setStartUrl] = useState(initialStartUrl)
  const [editIdc, setEditIdc] = useState(false)
  const [incognito, setIncognito] = useState(true)
  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const isRemote = isRemoteAccess()

  const clearPollTimer = () => {
    if (pollTimerRef.current) {
      clearTimeout(pollTimerRef.current)
      pollTimerRef.current = null
    }
  }

  useEffect(() => {
    if (open) {
      setMethod(initialMethod)
      setStep('select')
      setSocialSession(null)
      setIdcSession(null)
      setCallbackUrl('')
      setNextUrl('')
      setStartUrl(initialStartUrl)
      setRegion(initialRegion)
      setEditIdc(false)
    }
    return () => clearPollTimer()
  }, [open, initialMethod, initialStartUrl, initialRegion])

  const copyText = async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text)
      toast.success(`${label}已复制`)
    } catch {
      toast.error('复制失败，请手动复制')
    }
  }

  const handleOpenChange = (v: boolean) => {
    if (!v) {
      clearPollTimer()
      setStep('select')
      setSocialSession(null)
      setIdcSession(null)
      setCallbackUrl('')
      setNextUrl('')
    }
    onOpenChange(v)
  }

  const scheduleSocialPoll = (sessionId: string) => {
    clearPollTimer()
    pollTimerRef.current = setTimeout(async () => {
      try {
        const result = await pollSocialRelogin(credentialId, sessionId)
        if (result.status === 'pending') {
          scheduleSocialPoll(sessionId)
        } else if (result.status === 'continue') {
          const next = getContinueUrl(result)
          if (next) setNextUrl(next)
          scheduleSocialPoll(sessionId)
        } else if (result.status === 'success') {
          setStep('done')
          onSuccess()
          toast.success(`凭据 #${result.credentialId} Token 已更新`)
        } else {
          toast.error('会话已过期，请重试')
          setStep('select')
        }
      } catch (e) {
        toast.error('轮询失败：' + extractErrorMessage(e))
        scheduleSocialPoll(sessionId)
      }
    }, POLL_INTERVAL_MS)
  }

  // 设备授权按服务端给的 interval 轮询（通常 5s），过快会被 OIDC 返回 slow_down
  const scheduleIdcPoll = (sessionId: string, intervalSec: number) => {
    clearPollTimer()
    pollTimerRef.current = setTimeout(async () => {
      try {
        const result = await pollIdcRelogin(credentialId, sessionId)
        if (result.status === 'pending') {
          scheduleIdcPoll(sessionId, intervalSec)
        } else if (result.status === 'success') {
          setStep('done')
          onSuccess()
          toast.success(`凭据 #${result.credentialId} Token 已更新`)
        } else {
          toast.error('会话已过期，请重试')
          setStep('select')
        }
      } catch (e) {
        toast.error('轮询失败：' + extractErrorMessage(e))
        scheduleIdcPoll(sessionId, intervalSec)
      }
    }, Math.max(intervalSec, 1) * 1000)
  }

  const handleStartSocial = async () => {
    setIsStarting(true)
    try {
      const resp = await startSocialRelogin(credentialId, {})
      setSocialSession(resp)
      setStep('waiting')
      window.open(resp.portalUrl, '_blank')
      scheduleSocialPoll(resp.sessionId)
    } catch (e) {
      toast.error('发起 Social 重新登录失败：' + extractErrorMessage(e))
    } finally {
      setIsStarting(false)
    }
  }

  const handleStartIdc = async () => {
    if (!region.trim()) {
      toast.error('请填写 SSO 区域')
      return
    }
    setIsStarting(true)
    try {
      const resp = await startIdcRelogin(credentialId, {
        region: region.trim(),
        startUrl: startUrl.trim() || undefined,
      })
      setIdcSession(resp)
      setStep('waiting')
      const openUrl = resp.verificationUriComplete || resp.verificationUri
      if (incognito) {
        // 浏览器无法用脚本打开无痕窗口：复制链接让用户在无痕窗口粘贴，避免误用当前已登录的账号
        await copyText(openUrl, '验证链接')
      } else {
        window.open(openUrl, '_blank')
      }
      scheduleIdcPoll(resp.sessionId, resp.pollInterval)
    } catch (e) {
      toast.error('发起 IdC 重新登录失败：' + extractErrorMessage(e))
    } finally {
      setIsStarting(false)
    }
  }

  const handleCompleteSocial = async () => {
    if (!socialSession) return
    const parsed = parseCallbackUrl(callbackUrl)
    if (!parsed) {
      toast.error('URL 格式无效，请复制完整的地址栏 URL')
      return
    }
    clearPollTimer()
    setIsCompleting(true)
    try {
      const result = await completeSocialRelogin(credentialId, socialSession.sessionId, {
        code: parsed.code,
        state: parsed.state,
        loginOption: parsed.loginOption || undefined,
        path: parsed.path,
        issuerUrl: parsed.issuerUrl,
        clientId: parsed.clientId,
        scopes: parsed.scopes,
        loginHint: parsed.loginHint,
      })
      if (result.status === 'success') {
        setStep('done')
        onSuccess()
        toast.success(`凭据 #${result.credentialId} Token 已更新`)
      } else if (result.status === 'continue') {
        const next = getContinueUrl(result)
        if (next) {
          setNextUrl(next)
          toast.success('已获取二段登录链接')
        }
        scheduleSocialPoll(socialSession.sessionId)
      } else if (result.status === 'expired') {
        toast.error('会话已过期，请重新发起')
        setStep('select')
      } else {
        scheduleSocialPoll(socialSession.sessionId)
      }
    } catch (e) {
      toast.error('完成登录失败：' + extractErrorMessage(e))
      scheduleSocialPoll(socialSession.sessionId)
    } finally {
      setIsCompleting(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>
            重新登录 · {credential?.email || `凭据 #${credentialId}`}
          </DialogTitle>
          <DialogDescription>
            通过浏览器重新授权，原地更新该凭据的 Token（不新建凭据，优先级和统计保留）
          </DialogDescription>
        </DialogHeader>

        {step === 'select' && (
          <div className="space-y-4">
            {credential?.email && (
              <div className="rounded-md border border-amber-300 bg-amber-50 p-2.5 text-sm dark:border-amber-900 dark:bg-amber-950/40">
                请使用 <span className="font-medium">{credential.email}</span> 登录。
                用其他账号授权会把该凭据替换成另一个账号。
              </div>
            )}

            {method === 'idc' && !editIdc ? (
              <div className="rounded-lg border p-3 text-sm space-y-1">
                <div className="flex items-center justify-between">
                  <span className="font-medium">
                    {isEnterpriseIdc ? 'Enterprise IAM Identity Center SSO' : 'AWS Builder ID'}
                  </span>
                  <button
                    type="button"
                    className="text-xs text-muted-foreground hover:text-foreground hover:underline"
                    onClick={() => setEditIdc(true)}
                  >
                    修改
                  </button>
                </div>
                <div className="text-muted-foreground break-all">
                  {startUrl || BUILDER_ID_START_URL} · {region}
                </div>
              </div>
            ) : (
              <>
                <div className="grid gap-2">
                  <button
                    type="button"
                    className={`rounded-lg border p-3 text-left ${method === 'idc' ? 'border-primary bg-accent/50' : ''}`}
                    onClick={() => setMethod('idc')}
                  >
                    <div className="font-medium">IdC 设备授权</div>
                    <div className="text-sm text-muted-foreground">企业 IAM Identity Center / Builder ID</div>
                  </button>
                  <button
                    type="button"
                    className={`rounded-lg border p-3 text-left ${method === 'social' ? 'border-primary bg-accent/50' : ''}`}
                    onClick={() => setMethod('social')}
                  >
                    <div className="font-medium">Social / 企业 SSO</div>
                    <div className="text-sm text-muted-foreground">Portal PKCE 登录（Google / GitHub / Azure AD）</div>
                  </button>
                </div>

                {method === 'idc' && (
                  <div className="space-y-2">
                    <label className="text-sm font-medium" htmlFor="relogin-start-url">SSO Start URL</label>
                    <Input
                      id="relogin-start-url"
                      value={startUrl}
                      onChange={(e) => setStartUrl(e.target.value)}
                      placeholder={`留空使用 Builder ID（${BUILDER_ID_START_URL}）`}
                    />
                    <label className="text-sm font-medium" htmlFor="relogin-region">SSO 区域</label>
                    <Input
                      id="relogin-region"
                      value={region}
                      onChange={(e) => setRegion(e.target.value)}
                      placeholder="ap-southeast-1"
                    />
                  </div>
                )}
              </>
            )}

            {method === 'idc' && (
              <label className="flex items-start gap-2 rounded-lg border bg-muted/40 p-3 cursor-pointer">
                <input
                  type="checkbox"
                  checked={incognito}
                  onChange={(e) => setIncognito(e.target.checked)}
                  className="mt-0.5 h-4 w-4 shrink-0 accent-primary"
                />
                <span className="text-sm">
                  <span className="font-medium">用无痕窗口登录（推荐多账号时使用）</span>
                  <span className="mt-0.5 block text-xs text-muted-foreground">
                    发起后自动复制验证链接，在无痕窗口（⌘+Shift+N）粘贴打开，避免浏览器沿用已登录的其他账号。
                  </span>
                </span>
              </label>
            )}

            <DialogFooter>
              <Button variant="outline" onClick={() => handleOpenChange(false)}>
                取消
              </Button>
              <Button
                disabled={isStarting}
                onClick={() => (method === 'social' ? handleStartSocial() : handleStartIdc())}
              >
                {isStarting ? <Loader2 className="h-4 w-4 mr-2 animate-spin" /> : null}
                开始重新登录
              </Button>
            </DialogFooter>
          </div>
        )}

        {step === 'waiting' && (
          <div className="space-y-4">
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              等待浏览器授权完成…
            </div>

            {method === 'idc' && idcSession && (
              <div className="rounded-md border p-3 text-sm space-y-2">
                {incognito && (
                  <p className="text-muted-foreground">
                    验证链接已复制，请在无痕窗口粘贴打开
                    {credential?.email ? `，并用 ${credential.email} 登录` : ''}。
                  </p>
                )}
                <div className="flex items-center gap-2">
                  验证码：<span className="font-mono text-lg font-semibold tracking-widest">{idcSession.userCode}</span>
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7"
                    onClick={() => copyText(idcSession.userCode, '验证码')}
                    aria-label="复制验证码"
                  >
                    <Copy className="h-3.5 w-3.5" />
                  </Button>
                </div>
                <div className="flex flex-wrap gap-3">
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() =>
                      copyText(idcSession.verificationUriComplete || idcSession.verificationUri, '验证链接')
                    }
                  >
                    <Copy className="h-3.5 w-3.5 mr-1" />
                    复制验证链接
                  </Button>
                  <a
                    href={idcSession.verificationUriComplete || idcSession.verificationUri}
                    target="_blank"
                    rel="noreferrer"
                    className="inline-flex items-center text-primary hover:underline"
                  >
                    在当前浏览器打开 <ExternalLink className="h-3 w-3 ml-1" />
                  </a>
                </div>
              </div>
            )}

            {method === 'social' && socialSession && (
              <div className="space-y-2">
                <a
                  href={socialSession.portalUrl}
                  target="_blank"
                  rel="noreferrer"
                  className="inline-flex items-center text-sm text-primary hover:underline"
                >
                  重新打开登录页 <ExternalLink className="h-3 w-3 ml-1" />
                </a>
                {nextUrl && (
                  <a
                    href={nextUrl}
                    target="_blank"
                    rel="noreferrer"
                    className="block text-sm text-primary hover:underline"
                  >
                    打开二段企业 SSO 链接
                  </a>
                )}
                {(isRemote || true) && (
                  <div className="space-y-2 pt-2 border-t">
                    <p className="text-xs text-muted-foreground">
                      远程访问或未自动回调时，请粘贴浏览器地址栏的回调 URL：
                    </p>
                    <Input
                      value={callbackUrl}
                      onChange={(e) => setCallbackUrl(e.target.value)}
                      placeholder="http://127.0.0.1:xxxx/oauth/callback?code=..."
                    />
                    <Button
                      size="sm"
                      disabled={isCompleting || !callbackUrl.trim()}
                      onClick={handleCompleteSocial}
                    >
                      {isCompleting ? <Loader2 className="h-4 w-4 mr-2 animate-spin" /> : null}
                      手动完成
                    </Button>
                  </div>
                )}
              </div>
            )}

            <DialogFooter>
              <Button
                variant="outline"
                onClick={() => {
                  clearPollTimer()
                  setStep('select')
                }}
              >
                返回
              </Button>
            </DialogFooter>
          </div>
        )}

        {step === 'done' && (
          <div className="space-y-4 py-4 text-center">
            <CheckCircle className="h-10 w-10 text-green-500 mx-auto" />
            <p className="font-medium">Token 已更新</p>
            <DialogFooter>
              <Button onClick={() => handleOpenChange(false)}>完成</Button>
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  )
}
