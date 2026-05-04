import {
  createContext,
  createSignal,
  type JSX,
  useContext,
} from "solid-js";
import ko from "./i18n/ko.json";
import en from "./i18n/en.json";

type Locale = "ko" | "en";

const dict: Record<Locale, Record<string, string>> = { ko, en };

interface I18nCtx {
  t: (key: string) => string;
  locale: () => Locale;
  setLocale: (l: Locale) => void;
}

const Ctx = createContext<I18nCtx>();

function detectInitialLocale(): Locale {
  const stored = (() => {
    try {
      return localStorage.getItem("locale") as Locale | null;
    } catch {
      return null;
    }
  })();
  if (stored === "ko" || stored === "en") return stored;
  const navigator = (globalThis as any).navigator as
    | { language?: string }
    | undefined;
  return navigator?.language?.startsWith("en") ? "en" : "ko";
}

export function I18nProvider(props: { children: JSX.Element }) {
  const [locale, setLocaleSignal] = createSignal<Locale>(detectInitialLocale());
  const setLocale = (l: Locale) => {
    setLocaleSignal(l);
    try {
      localStorage.setItem("locale", l);
    } catch {
      // 브라우저 외부 환경에선 무시
    }
  };
  const t = (key: string): string => {
    return dict[locale()][key] ?? key;
  };
  return <Ctx.Provider value={{ t, locale, setLocale }}>{props.children}</Ctx.Provider>;
}

export function useI18n(): I18nCtx {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useI18n must be inside I18nProvider");
  return ctx;
}
