// Tauri plugin-clipboard-manager shim — 브라우저 Clipboard API 사용.

export async function writeText(text: string): Promise<void> {
 if (navigator.clipboard?.writeText) {
 await navigator.clipboard.writeText(text);
 return;
}
 // Fallback (구형 브라우저 / non-secure context).
 const ta = document.createElement("textarea");
 ta.value = text;
 ta.style.position = "fixed";
 ta.style.opacity = "0";
 document.body.appendChild(ta);
 ta.select();
 try {
 document.execCommand("copy");
} finally {
 document.body.removeChild(ta);
}
}

export async function clear(): Promise<void> {
 // 브라우저는 빈 문자열로 덮어쓰는 것 외 클리어 수단 없음.
 await writeText("");
}
