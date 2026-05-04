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
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', init);
} else {
  init();
}
