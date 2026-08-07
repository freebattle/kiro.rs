import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react-swc'
import fs from 'fs'
import path from 'path'

// 版本号统一以根目录 Cargo.toml 的 [package].version 为准
function readCargoVersion(): string {
  const cargoToml = path.resolve(__dirname, '../Cargo.toml')
  try {
    const content = fs.readFileSync(cargoToml, 'utf-8')
    const packageSection = content.split(/^\[/m).find((section) => section.startsWith('package]'))
    const match = packageSection?.match(/^\s*version\s*=\s*"([^"]+)"/m)
    if (match) {
      return match[1]
    }
    throw new Error('未找到 [package].version')
  } catch (error) {
    console.warn(`[vite] 读取 ${cargoToml} 版本号失败：${(error as Error).message}`)
    return 'unknown'
  }
}

export default defineConfig({
  plugins: [react()],
  define: {
    __APP_VERSION__: JSON.stringify(readCargoVersion()),
  },
  base: '/admin/',
  resolve: {
    alias: {
      '@': path.resolve(__dirname, './src'),
    },
  },
  server: {
    proxy: {
      '/api': {
        target: 'http://localhost:8080',
        changeOrigin: true,
      },
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
})
