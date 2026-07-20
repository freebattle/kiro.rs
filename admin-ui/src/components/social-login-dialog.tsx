import { useState, useEffect, useRef } from 'react'
import { toast } from 'sonner'
import { ExternalLink, CheckCircle, Loader2, Copy, Check } from 'lucide-react'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '@/components/ui/dialog'
import { Button } from '@/components/ui/button'
import { Input } from '@/components/ui/input'
import { startSocialLogin, pollSocialLogin, completeSocialLogin } from '@/api/credentials'
import type { StartSocialLoginResponse } from '@/types/api'
import { extractErrorMessage } from '@/lib/utils'

interface SocialLoginDialogProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  onSuccess: () => void
}

type Step = 'form' | 'waiting' | 'done'
type CopyState = 'idle' | 'copied' | 'manual'

const POLL_INTERVAL_MS = 2000

// 检查是否为远程访问（非本机）
const isRemoteAccess = () =>
  window.location.hostname !== 'localhost' && window.location.hostname !== '127.0.0.1'

// 从回调 URL 字符串中提取 OAuth 参数
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

async function getClipboardWritePermission(): Promise<PermissionState | 'unsupported'> {
  if (!navigator.permissions?.query) return 'unsupported'
  try {
    const status = await navigator.permissions.query({ name: 'clipboard-write' as PermissionName })
    return status.state
  } catch {
    return 'unsupported'
  }
}

export function SocialLoginDialog({ open, onOpenChange, onSuccess }: SocialLoginDialogProps) {
  const [step, setStep] = useState<Step>('form')
  const [email, setEmail] = useState('')
  const [incognito, setIncognito] = useState(false)
  const [copyState, setCopyState] = useState<CopyState>('idle')
  const [isStarting, setIsStarting] = useState(false)
  const [isCompleting, setIsCompleting] = useState(false)
  const [session, setSession] = useState<StartSocialLoginResponse | null>(null)
  const [credentialId, setCredentialId] = useState<number | null>(null)
  const [callbackUrl, setCallbackUrl] = useState('')
  const [nextUrl, setNextUrl] = useState('')
  const pollTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const loginLinkRef = useRef<HTMLTextAreaElement | null>(null)
  const isRemote = isRemoteAccess()

  const clearPollTimer = () => {
    if (pollTimerRef.current) {
      clearTimeout(pollTimerRef.current)
      pollTimerRef.current = null
    }
  }

  useEffect(() => {
    return () => clearPollTimer()
  }, [])

  const handleOpenChange = (v: boolean) => {
    if (!v) {
      clearPollTimer()
      setStep('form')
      setSession(null)
      setCredentialId(null)
      setIsStarting(false)
      setIsCompleting(false)
      setCallbackUrl('')
      setNextUrl('')
      setCopyState('idle')
    }
    onOpenChange(v)
  }

  const selectLoginLink = () => {
    window.requestAnimationFrame(() => {
      loginLinkRef.current?.focus()
      loginLinkRef.current?.select()
    })
  }

  const handleCopyLink = async (url: string) => {
    if (!window.isSecureContext || !navigator.clipboard?.writeText) {
      setCopyState('manual')
      selectLoginLink()
      toast.error('当前访问地址不是 HTTPS/localhost，浏览器不允许网页直接写入剪贴板')
      return
    }

    try {
      const permission = await getClipboardWritePermission()
      if (permission === 'denied') {
        setCopyState('manual')
        selectLoginLink()
        toast.error('Chrome 已拒绝该站点写入剪贴板，请在地址栏权限设置中允许后重试')
        return
      }

      await navigator.clipboard.writeText(url)
      setCopyState('copied')
      toast.success('链接已复制')
      setTimeout(() => setCopyState('idle'), 2000)
    } catch {
      setCopyState('manual')
      selectLoginLink()
      toast.error('浏览器拒绝写入剪贴板，已选中链接，请按 Ctrl+C 复制')
    }
  }

  const handleOpenLink = (url: string) => {
    const opened = window.open(url, '_blank')
    if (!opened) {
      void handleCopyLink(url)
      toast.error('浏览器拦截了新窗口，链接已保留在下方')
    }
  }

  const handleStart = async () => {
    setIsStarting(true)
    // 无痕模式：浏览器不允许 JS 直接开无痕窗口，改为复制链接让用户手动在无痕窗口打开，
    // 因此不预开 about:blank（避免在当前登录态里误开）。
    const loginWindow = incognito ? null : window.open('about:blank', '_blank')
    try {
      const resp = await startSocialLogin({
        email: email.trim() || undefined,
      })
      setSession(resp)
      setStep('waiting')
      if (incognito) {
        // 无痕：优先写入剪贴板；被浏览器策略拒绝时会选中下方链接。
        await handleCopyLink(resp.portalUrl)
      } else if (loginWindow) {
        loginWindow.location.href = resp.portalUrl
      } else {
        window.open(resp.portalUrl, '_blank')
      }
      // 回调可能自动投递，也可能需要手动粘贴；始终轮询以便及时展示企业 SSO 二段链接。
      schedulePoll(resp.sessionId)
    } catch (e) {
      loginWindow?.close()
      toast.error('发起登录失败：' + extractErrorMessage(e))
    } finally {
      setIsStarting(false)
    }
  }

  const schedulePoll = (sessionId: string) => {
    clearPollTimer()
    pollTimerRef.current = setTimeout(async () => {
      try {
        const result = await pollSocialLogin(sessionId)
        if (result.status === 'pending') {
          schedulePoll(sessionId)
        } else if (result.status === 'continue') {
          const next = getContinueUrl(result)
          if (!next) {
            toast.error('后端返回了二段登录状态，但没有返回链接')
            schedulePoll(sessionId)
            return
          }
          setNextUrl(next)
          setCopyState('idle')
          toast.success('已获取二段登录链接，请点击打开或复制')
          schedulePoll(sessionId)
        } else if (result.status === 'success') {
          setCredentialId(result.credentialId)
          setStep('done')
          onSuccess()
          toast.success(`登录成功，已添加凭据 #${result.credentialId}`)
        } else {
          toast.error('会话已过期，请重新发起登录')
          setStep('form')
          setSession(null)
        }
      } catch (e) {
        toast.error('轮询失败：' + extractErrorMessage(e))
        schedulePoll(sessionId)
      }
    }, POLL_INTERVAL_MS)
  }

  const handleCompleteManually = async () => {
    if (!session) return
    const parsed = parseCallbackUrl(callbackUrl)
    if (!parsed) {
      toast.error('URL 格式无效，请复制完整的地址栏 URL')
      return
    }
    clearPollTimer()
    setIsCompleting(true)
    try {
      const result = await completeSocialLogin(session.sessionId, {
        code: parsed.code,
        state: parsed.state,
        loginOption: parsed.loginOption || undefined,
        path: parsed.path,
        issuerUrl: parsed.issuerUrl,
        clientId: parsed.clientId,
        scopes: parsed.scopes,
        loginHint: parsed.loginHint,
      })
      if (result.status === 'continue') {
        const next = getContinueUrl(result)
        if (!next) {
          toast.error('后端返回了二段登录状态，但没有返回链接')
          return
        }
        setNextUrl(next)
        setCopyState('idle')
        setCallbackUrl('')
        toast.success('已获取二段登录链接，请点击打开或复制')
        schedulePoll(session.sessionId)
      } else if (result.status === 'success') {
        setCredentialId(result.credentialId)
        setStep('done')
        onSuccess()
        toast.success(`登录成功，已添加凭据 #${result.credentialId}`)
      } else {
        toast.error('会话已过期，请重新发起登录')
        setStep('form')
        setSession(null)
      }
    } catch (e) {
      toast.error('完成登录失败：' + extractErrorMessage(e))
    } finally {
      setIsCompleting(false)
    }
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Kiro Hosted 登录（Google / GitHub / 企业 SSO）</DialogTitle>
          <DialogDescription>
            通过 Kiro 网页端完成账号登录；企业账号会进入 Entra ID / Azure AD 二段授权。
          </DialogDescription>
        </DialogHeader>

        {step === 'form' && (
          <div className="space-y-4 py-2">
            <div className="space-y-1.5">
              <label htmlFor="social-email" className="text-sm font-medium">邮箱（可选）</label>
              <Input
                id="social-email"
                placeholder="user@example.com"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
            </div>
            <label className="flex items-start gap-2 rounded-lg border bg-muted/40 p-3 cursor-pointer">
              <input
                type="checkbox"
                checked={incognito}
                onChange={(e) => setIncognito(e.target.checked)}
                className="mt-0.5 h-4 w-4 shrink-0 accent-primary"
              />
              <span className="text-sm">
                <span className="font-medium">使用无痕窗口登录</span>
                <span className="mt-0.5 block text-xs text-muted-foreground">
                  发起后复制登录链接，自行用浏览器无痕/隐身窗口（Ctrl+Shift+N）打开，
                  避免与当前已登录的 Google / GitHub 账号串号。
                </span>
              </span>
            </label>
          </div>
        )}

        {step === 'waiting' && session && (
          <div className="space-y-4 py-2">
            {nextUrl ? (
              <div className="rounded-lg border bg-muted/50 p-4 space-y-3">
                <p className="text-sm font-medium">二段登录链接</p>
                <textarea
                  ref={loginLinkRef}
                  readOnly
                  value={nextUrl}
                  onFocus={(e) => e.currentTarget.select()}
                  className="flex min-h-[72px] w-full resize-none rounded-md border border-input bg-background px-3 py-2 text-xs font-mono text-muted-foreground focus-visible:outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/30"
                />
                <div className="flex flex-wrap items-center gap-2">
                  <Button size="sm" onClick={() => handleOpenLink(nextUrl)}>
                    <ExternalLink className="h-3.5 w-3.5" />
                    打开二段链接
                  </Button>
                  <Button size="sm" variant="outline" onClick={() => handleCopyLink(nextUrl)}>
                    {copyState === 'copied' ? (
                      <Check className="h-3.5 w-3.5" />
                    ) : (
                      <Copy className="h-3.5 w-3.5" />
                    )}
                    {copyState === 'copied' ? '已复制' : '复制链接'}
                  </Button>
                  {copyState === 'manual' && (
                    <span className="text-xs text-muted-foreground">
                      链接已选中，可直接按 Ctrl+C
                    </span>
                  )}
                </div>
              </div>
            ) : incognito ? (
              <div className="rounded-lg border bg-muted/50 p-4 space-y-3">
                <p className="text-sm text-muted-foreground">
                  {copyState === 'copied' ? '登录链接已复制。' : '复制登录链接后，'}
                  请新开一个<span className="font-medium text-foreground">无痕 / 隐身窗口</span>
                  （Ctrl+Shift+N，Safari 为 ⌘+Shift+N），粘贴打开并完成授权。
                </p>
                <textarea
                  ref={loginLinkRef}
                  readOnly
                  value={session.portalUrl}
                  onFocus={(e) => e.currentTarget.select()}
                  className="flex min-h-[72px] w-full resize-none rounded-md border border-input bg-background px-3 py-2 text-xs font-mono text-muted-foreground focus-visible:outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/30"
                />
                <div className="flex items-center gap-2">
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => handleCopyLink(session.portalUrl)}
                  >
                    {copyState === 'copied' ? (
                      <Check className="h-3.5 w-3.5" />
                    ) : (
                      <Copy className="h-3.5 w-3.5" />
                    )}
                    {copyState === 'copied' ? '已复制' : '复制登录链接'}
                  </Button>
                  {copyState === 'manual' && (
                    <span className="text-xs text-muted-foreground">
                      链接已选中，可直接按 Ctrl+C
                    </span>
                  )}
                </div>
              </div>
            ) : (
              <div className="rounded-lg border bg-muted/50 p-4 space-y-3">
                <p className="text-sm text-muted-foreground">
                  浏览器应已自动打开 Kiro 登录页，请完成授权。
                </p>
                <a
                  href={session.portalUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="inline-flex items-center gap-1.5 text-sm font-medium text-primary hover:underline"
                >
                  重新打开登录页
                  <ExternalLink className="h-3.5 w-3.5" />
                </a>
              </div>
            )}

            {!isRemote && (
              <div className="flex items-center gap-2 text-sm text-muted-foreground">
                <Loader2 className="h-4 w-4 animate-spin" />
                正在等待登录完成…
              </div>
            )}

            <div className="space-y-2">
              <p className={isRemote ? 'text-sm text-amber-600 dark:text-amber-400' : 'text-sm text-muted-foreground'}>
                {nextUrl
                  ? '打开二段链接完成授权后，把最终回调 URL 粘贴到下方：'
                  : isRemote
                    ? '完成登录后，浏览器可能会跳转到 localhost 页面，请从地址栏复制完整 URL 粘贴到下方。企业 SSO 的第一段中间链接也可以直接粘贴：'
                    : '如果浏览器停在 localhost 页面，也可以把完整 URL 粘贴到下方。企业 SSO 的第一段中间链接可直接粘贴：'}
              </p>
              <textarea
                placeholder={nextUrl
                  ? "http://localhost:3128/oauth/callback?code=...&state=..."
                  : "http://localhost:3128/signin/callback?login_option=external_idp&issuer_url=...&client_id=..."}
                value={callbackUrl}
                onChange={(e) => setCallbackUrl(e.target.value)}
                disabled={isCompleting}
                className="flex min-h-[80px] w-full rounded-md border border-input bg-background px-3 py-2 text-xs font-mono placeholder:text-muted-foreground focus-visible:outline-none focus-visible:border-ring focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring/30 disabled:opacity-50"
              />
            </div>
          </div>
        )}

        {step === 'done' && (
          <div className="flex flex-col items-center gap-3 py-4">
            <CheckCircle className="h-10 w-10 text-green-500" />
            <p className="text-sm font-medium">登录成功</p>
            <p className="text-xs text-muted-foreground">凭据 #{credentialId} 已添加并启用</p>
          </div>
        )}

        <DialogFooter>
          {step === 'form' && (
            <Button onClick={handleStart} disabled={isStarting}>
              {isStarting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
              发起登录
            </Button>
          )}
          {step === 'waiting' && (
            <>
              <Button variant="outline" onClick={() => handleOpenChange(false)} disabled={isCompleting}>
                取消
              </Button>
              <Button
                onClick={handleCompleteManually}
                disabled={isCompleting || !callbackUrl.trim()}
              >
                {isCompleting && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                完成登录
              </Button>
            </>
          )}
          {step === 'done' && (
            <Button onClick={() => handleOpenChange(false)}>关闭</Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
