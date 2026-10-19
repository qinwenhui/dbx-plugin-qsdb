// DBX Host Bridge（window.dbxPlugin）的类型与薄封装。
// UI 只通过这里调 Sidecar / storage；**绝不直接 fetch QSDB**（沙箱无网络）。

export interface DbxColumn {
  name: string;
  tag: string | null;
  typeName: string;
  nullable: boolean;
  partition?: boolean;
}

export interface DbxTheme {
  appearance: "dark" | "light";
  tokens?: Record<string, string>;
}

export interface DbxContext {
  connectionId?: string;
  database?: string;
  schema?: string;
  workbenchId?: string;
  [key: string]: unknown;
}

export interface DbxStorage {
  get(key: string): Promise<unknown>;
  set(key: string, value: unknown): Promise<void>;
  delete(key: string): Promise<void>;
}

export interface DbxPluginApi {
  ready: Promise<DbxContext>;
  readonly context: DbxContext | undefined;
  readonly locale: string;
  readonly theme: DbxTheme | undefined;
  invoke<T = unknown>(method: string, params?: unknown, options?: { timeoutMs?: number }): Promise<T>;
  onEvent(listener: (message: DbxBridgeMessage) => void): () => void;
  onContext(listener: (context: DbxContext) => void): () => void;
  storage: DbxStorage;
}

export type DbxBridgeMessage = {
  type?: string;
  method?: string;
  params?: unknown;
  locale?: string;
  theme?: DbxTheme;
  [key: string]: unknown;
};

declare global {
  interface Window {
    dbxPlugin?: DbxPluginApi;
  }
}

export class InvokeError extends Error {
  readonly qsdbCode: string | null;
  readonly status: number | null;
  readonly retriable: boolean;

  constructor(message: string, data?: Record<string, unknown>) {
    super(message);
    this.name = "InvokeError";
    this.qsdbCode = (data?.qsdbCode as string) ?? null;
    this.status = (data?.status as number) ?? null;
    this.retriable = Boolean(data?.retriable);
  }

  get isConnectionGone(): boolean {
    return this.qsdbCode === "CONNECTION_NOT_FOUND";
  }
}

export function api(): DbxPluginApi {
  const a = window.dbxPlugin;
  if (!a) {
    throw new Error("window.dbxPlugin is unavailable: open this workbench inside DBX");
  }
  return a;
}

/** invoke + 错误规范化（DBX 桥的错误是字符串/Error，QSDB 业务码在 message/data 里）。 */
export async function invoke<T>(method: string, params?: unknown, options?: { timeoutMs?: number }): Promise<T> {
  try {
    return await api().invoke<T>(method, params, options);
  } catch (e) {
    if (e instanceof InvokeError) throw e;
    const err = e as Error;
    let data: Record<string, unknown> | undefined;
    let message = err?.message ?? String(e);
    // 宿主桥把 sidecar 的 error.message 透出；data 可能拼在 message 尾部
    const m = message.match(/\{[\s\S]*"qsdbCode"[\s\S]*\}/);
    if (m) {
      try {
        data = JSON.parse(m[0]) as Record<string, unknown>;
        message = message.slice(0, m.index).trim() || (data.qsdbCode as string);
      } catch {
        /* 解不出来就按原文显示 */
      }
    }
    throw new InvokeError(message, data);
  }
}

/** 订阅 sidecar 事件（qsdb/query/*）。 */
export function onQueryEvent(listener: (method: string, params: Record<string, unknown>) => void): () => void {
  return api().onEvent((message) => {
    if (typeof message.method === "string" && message.method.startsWith("qsdb/")) {
      listener(message.method, (message.params as Record<string, unknown>) ?? {});
    }
  });
}

/** 订阅宿主环境变化（theme/locale 推送也走事件通道）。 */
export function onEnvChange(listener: (env: { locale?: string; theme?: DbxTheme }) => void): () => void {
  return api().onEvent((message) => {
    if (message.type === "env") {
      listener({ locale: message.locale, theme: message.theme });
    }
  });
}
