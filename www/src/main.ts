// Common bootstrap: i18n toggle, active nav highlighting.
import './styles/main.css';

type Lang = 'ko' | 'en';
const STORAGE_KEY = 'oxg-lang';

function detectLang(): Lang {
  const saved = localStorage.getItem(STORAGE_KEY) as Lang | null;
  if (saved === 'ko' || saved === 'en') return saved;
  const nav = navigator.language.toLowerCase();
  return nav.startsWith('ko') ? 'ko' : 'en';
}

function setLang(lang: Lang) {
  document.documentElement.lang = lang;
  localStorage.setItem(STORAGE_KEY, lang);
  const btn = document.querySelector<HTMLButtonElement>('.lang-toggle');
  if (btn) btn.textContent = lang === 'ko' ? 'EN' : 'KO';
}

function highlightNav() {
  const path = window.location.pathname.replace(/\/$/, '') || '/';
  document.querySelectorAll<HTMLAnchorElement>('.nav a').forEach((a) => {
    const href = a.getAttribute('href') || '';
    const norm = href.replace(/\/$/, '') || '/';
    if (norm === path || (norm !== '/' && path.startsWith(norm))) {
      a.classList.add('active');
    }
  });
}

/**
 * `<pre class="code">` 의 자식 노드를 `\n` 기준으로 라인 단위로 재그룹.
 * `<span class="p">$</span>` 로 시작하는 라인은 cmd-row 로 감싸고 복사 버튼 추가.
 * 멀티라인(끝이 `\`) 명령은 다음 라인까지 묶어 한 명령으로 복사.
 *
 * DOM API 만 사용 — innerHTML 안 씀, 기존 syntax 하이라이트 span 유지.
 */
function setupCopyButtons() {
  document.querySelectorAll<HTMLPreElement>('pre.code').forEach((pre) => {
    if (pre.dataset.copyEnhanced === '1') return;
    pre.dataset.copyEnhanced = '1';

    // 1. 자식 노드를 `\n` 기준 라인 그룹으로 분리.
    const lines: Node[][] = [[]];
    pre.childNodes.forEach((node) => {
      if (node.nodeType === Node.TEXT_NODE && node.textContent && node.textContent.includes('\n')) {
        const parts = node.textContent.split('\n');
        parts.forEach((part, idx) => {
          if (part.length > 0) lines[lines.length - 1].push(document.createTextNode(part));
          if (idx < parts.length - 1) lines.push([]);
        });
      } else {
        lines[lines.length - 1].push(node);
      }
    });

    const lineIsPrompt = (nodes: Node[]) => {
      const first = nodes[0];
      return first instanceof Element && first.classList.contains('p');
    };
    const lineText = (nodes: Node[]) =>
      nodes.map((n) => n.textContent ?? '').join('');
    const endsWithBackslash = (nodes: Node[]) => lineText(nodes).trimEnd().endsWith('\\');

    // 2. 라인을 한 번씩 순회하며 prompt 라인은 그룹화 → cmd-row 로 감싼다.
    pre.replaceChildren();
    let i = 0;
    while (i < lines.length) {
      if (i > 0) pre.appendChild(document.createTextNode('\n'));
      const cur = lines[i];

      if (!lineIsPrompt(cur)) {
        cur.forEach((n) => pre.appendChild(n));
        i += 1;
        continue;
      }

      // 백슬래시 연속 라인 묶기.
      const group: Node[][] = [cur];
      while (endsWithBackslash(group[group.length - 1])) {
        if (i + group.length >= lines.length) break;
        group.push(lines[i + group.length]);
      }

      const cmd = group
        .map(lineText)
        .join('\n')
        .replace(/^\s*\$\s?/, '')
        .trim();

      const row = document.createElement('span');
      row.className = 'cmd-row';
      row.dataset.cmd = cmd;
      group.forEach((g, idx) => {
        if (idx > 0) row.appendChild(document.createTextNode('\n'));
        g.forEach((n) => row.appendChild(n));
      });

      const btn = document.createElement('button');
      btn.type = 'button';
      btn.className = 'copy-cmd';
      btn.textContent = '⧉';
      btn.title = '복사';
      btn.setAttribute('aria-label', '명령 복사');
      row.appendChild(btn);
      pre.appendChild(row);

      i += group.length;
    }
  });

  document.addEventListener('click', (e) => {
    const target = e.target as HTMLElement;
    if (!target.classList.contains('copy-cmd')) return;
    const row = target.closest<HTMLElement>('.cmd-row');
    if (!row) return;
    const cmd = row.dataset.cmd ?? '';
    void navigator.clipboard.writeText(cmd).then(() => {
      const orig = target.textContent;
      target.textContent = '✓';
      target.classList.add('copied');
      window.setTimeout(() => {
        target.textContent = orig;
        target.classList.remove('copied');
      }, 1200);
    });
  });
}

function init() {
  const lang = detectLang();
  setLang(lang);
  const btn = document.querySelector<HTMLButtonElement>('.lang-toggle');
  if (btn) {
    btn.addEventListener('click', () => {
      const cur = (document.documentElement.lang as Lang) || 'en';
      setLang(cur === 'ko' ? 'en' : 'ko');
    });
  }
  highlightNav();
  setupCopyButtons();
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', init);
} else {
  init();
}
