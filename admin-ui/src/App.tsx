import { useState, useEffect } from 'react'
import { storage } from '@/lib/storage'
import { LoginPage } from '@/components/login-page'
import { Dashboard } from '@/components/dashboard'
import { RequestLogPage } from '@/components/request-log-page'
import { UsageStatsPage } from '@/components/usage-stats-page'
import { Toaster } from '@/components/ui/sonner'

type Page = 'dashboard' | 'requests' | 'usage-stats'

function App() {
  const [isLoggedIn, setIsLoggedIn] = useState(false)
  const [currentPage, setCurrentPage] = useState<Page>('dashboard')

  useEffect(() => {
    // 检查是否已经有保存的 API Key
    if (storage.getApiKey()) {
      setIsLoggedIn(true)
    }
  }, [])

  const handleLogin = () => {
    setIsLoggedIn(true)
  }

  const handleLogout = () => {
    setIsLoggedIn(false)
    setCurrentPage('dashboard')
  }

  if (!isLoggedIn) {
    return (
      <>
        <LoginPage onLogin={handleLogin} />
        <Toaster position="top-right" />
      </>
    )
  }

  return (
    <>
      {currentPage === 'dashboard' ? (
        <Dashboard onLogout={handleLogout} onNavigate={setCurrentPage} />
      ) : currentPage === 'requests' ? (
        <RequestLogPage onBack={() => setCurrentPage('dashboard')} />
      ) : (
        <UsageStatsPage onBack={() => setCurrentPage('dashboard')} />
      )}
      <Toaster position="top-right" />
    </>
  )
}

export default App
