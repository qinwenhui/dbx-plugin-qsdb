// 剪贴板：navigator.clipboard 优先（沙箱 iframe 同源场景一般可用），
// 宿主 `host.copy` 兜底（宿主若实现了桥方法则二者都能走）。
// 返回是否成功——UI 据此给出"已复制"反馈，失败静默（不弹错误打断操作）。

import { api } from "./bridge";

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    /* 权限拒绝或不可用 → 试宿主桥 */
  }
  try {
    await api().invoke("host.copy", { text });
    return true;
  } catch {
    return false;
  }
}
