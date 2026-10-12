import React, { createContext, useContext, useState, useEffect } from "react";
import { Language, Translations } from "./types";
import { ru } from "./ru";
import { en } from "./en";
import { uk } from "./uk";

interface I18nContextValue {
  language: Language;
  setLanguage: (lang: Language) => void;
  t: Translations;
}

const translations: Record<Language, Translations> = {
  ru,
  en,
  uk,
};

const I18nContext = createContext<I18nContextValue>({
  language: "ru",
  setLanguage: () => {},
  t: ru,
});

const STORAGE_KEY = "zapret2_language";

export const I18nProvider: React.FC<{ children: React.ReactNode }> = ({ children }) => {
  const [language, setLanguageState] = useState<Language>(() => {
    try {
      const saved = localStorage.getItem(STORAGE_KEY) as Language | null;
      if (saved && (saved === "ru" || saved === "en" || saved === "uk")) {
        return saved;
      }
    } catch {}
    return "ru"; // Russian by default
  });

  const setLanguage = (lang: Language) => {
    setLanguageState(lang);
    try {
      localStorage.setItem(STORAGE_KEY, lang);
    } catch {}
  };

  const t = translations[language];

  return (
    <I18nContext.Provider value={{ language, setLanguage, t }}>
      {children}
    </I18nContext.Provider>
  );
};

export const useI18n = () => useContext(I18nContext);
