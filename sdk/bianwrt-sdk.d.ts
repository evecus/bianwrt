// bianwrt 官方插件 SDK 类型定义（配合 <script src="bianwrt-sdk.js"></script> 使用）

export interface BianwrtInfo {
  /** 当前插件名 */
  plugin: string
  /** 面板主题，目前固定 'dark' */
  theme: string
}

export interface ApiRequestOptions {
  /** 请求体（POST/PUT/PATCH） */
  data?: unknown
  /** URL 查询参数（GET/DELETE） */
  params?: Record<string, unknown>
}

export interface BianwrtApi {
  request<T = unknown>(method: string, path: `/api/${string}`, opts?: ApiRequestOptions): Promise<T>
  get<T = unknown>(path: `/api/${string}`, params?: Record<string, unknown>): Promise<T>
  post<T = unknown>(path: `/api/${string}`, data?: unknown): Promise<T>
  put<T = unknown>(path: `/api/${string}`, data?: unknown): Promise<T>
  patch<T = unknown>(path: `/api/${string}`, data?: unknown): Promise<T>
  delete<T = unknown>(path: `/api/${string}`, params?: Record<string, unknown>): Promise<T>
}

export interface BianwrtStorage {
  get<T = unknown>(key: string): Promise<T | null>
  set(key: string, value: unknown): Promise<boolean>
  remove(key: string): Promise<boolean>
  keys(): Promise<string[]>
  clear(): Promise<boolean>
}

export interface BianwrtNotify {
  (type: 'success' | 'info' | 'warning' | 'error', text: string): Promise<boolean>
  success(text: string): Promise<boolean>
  info(text: string): Promise<boolean>
  warning(text: string): Promise<boolean>
  error(text: string): Promise<boolean>
}

export interface BianwrtSDK {
  version: string
  /** 面板桥就绪（收到 hello 或 PING 回复） */
  ready: Promise<BianwrtInfo>
  readonly info: BianwrtInfo
  api: BianwrtApi
  storage: BianwrtStorage
  notify: BianwrtNotify
  /** 生成 /plugins/<name>/<path> 形式的插件内绝对 URL */
  pluginUrl(path?: string): string
  /** 请求插件自身后端（仅 proxy 模式可用，面板已放开 CORS） */
  fetch(path?: string, opts?: RequestInit): Promise<Response>
  on(event: 'hello', fn: (info: BianwrtInfo) => void): () => void
  off(event: string, fn: (...args: unknown[]) => void): void
}

declare global {
  interface Window { Bianwrt: BianwrtSDK }
  const Bianwrt: BianwrtSDK
}
