import { useEffect, useRef, useState } from 'react'
import { toast } from 'sonner'
import { CheckCircle, ExternalLink, Loader2 } from 'lucide-react'
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
import type { StartIdcLoginResponse, StartSocialLoginResponse } from '@/types/api'
import { extractErrorMessage } from '@/lib/utils'

interface ReloginDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  credentialId: number
  authMethod: string | null
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
  credentialId,
  authMethod,
  onSuccess,
}: ReloginDialogProps) {
  const initialMethod: Method =
    authMethod === 'idc' || authMethod === 'builder-id' || authMethod === 'iam'
      ? 'idc'
      : 'social'

  const [method, setMethod] = useState<Method>(initialMethod)
  const [step, setStep] = useState<Step>('select')
  const [isStarting, setIsStarting] = useState(false)
  const [isCompleting, setIsCompleting] = useState(false)
  const [callbackUrl, setCallbackUrl] = useState('')
  const [nextUrl, setNextUrl] = useState('')
  const [socialSession, setSocialSession] = useState<StartSocialLoginResponse | null>(null)
  const [idcSession, setIdcSession] = useState<StartIdcLoginResponse | null>(null)
  const [region, setRegion] = useState('us-east-1')
  const [startUrl, setStartUrl] = useState('')
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
    }
    return () => clearPollTimer()
  }, [open, initialMethod])

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

  const scheduleIdcPoll = (sessionId: string) => {
    clearPollTimer()
    pollTimerRef.current = setTimeout(async () => {
      try {
        const result = await pollIdcRelogin(credentialId, sessionId)
        if (result.status === 'pending') {
          scheduleIdcPoll(sessionId)
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
        scheduleIdcPoll(sessionId)
      }
    }, POLL_INTERVAL_MS)
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
      toast.error('请填写 Region')
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
      window.open(openUrl, '_blank')
      scheduleIdcPoll(resp.sessionId)
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
          <DialogTitle>重新登录 · 凭据 #{credentialId}</DialogTitle>
          <DialogDescription>
            Token 失效时通过浏览器重新授权，更新该凭据的 refreshToken（不会新建凭据）
          </DialogDescription>
        </DialogHeader>

        {step === 'select' && (
          <div className="space-y-4">
            <div className="grid gap-2">
              <button
                type="button"
                className={`rounded-lg border p-3 text-left ${method === 'social' ? 'border-primary bg-accent/50' : ''}`}
                onClick={() => setMethod('social')}
              >
                <div className="font-medium">Social / 企业 SSO</div>
                <div className="text-sm text-muted-foreground">Portal PKCE 登录（Google / GitHub / Azure AD）</div>
              </button>
              <button
                type="button"
                className={`rounded-lg border p-3 text-left ${method === 'idc' ? 'border-primary bg-accent/50' : ''}`}
                onClick={() => setMethod('idc')}
              >
                <div className="font-medium">IdC 设备授权</div>
                <div className="text-sm text-muted-foreground">Builder ID / 企业 IAM Identity Center</div>
              </button>
            </div>

            {method === 'idc' && (
              <div className="space-y-2">
                <label className="text-sm font-medium">Region</label>
                <Input value={region} onChange={(e) => setRegion(e.target.value)} placeholder="us-east-1" />
                <label className="text-sm font-medium">Start URL（企业 IdC 可选）</label>
                <Input
                  value={startUrl}
                  onChange={(e) => setStartUrl(e.target.value)}
                  placeholder="留空使用 Builder ID 默认"
                />
              </div>
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
              <div className="rounded-md border p-3 text-sm space-y-1">
                <div>
                  验证码：<span className="font-mono font-semibold">{idcSession.userCode}</span>
                </div>
                <a
                  href={idcSession.verificationUriComplete || idcSession.verificationUri}
                  target="_blank"
                  rel="noreferrer"
                  className="inline-flex items-center text-primary hover:underline"
                >
                  打开验证页面 <ExternalLink className="h-3 w-3 ml-1" />
                </a>
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
