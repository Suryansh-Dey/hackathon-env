'use client';
// Minimal client-side auth: a bearer token in localStorage. In dev the token
// comes from POST /dev/token (phone-only, no OTP); in prod it would be a
// Cognito access token from the Amplify Auth module. Static-export safe.
import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from 'react';

const KEY = 'kabados.token';

interface AuthState {
  token: string | null;
  sub: string | null;
  setToken: (token: string | null, sub?: string | null) => void;
  signOut: () => void;
}

const Ctx = createContext<AuthState | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [token, setTok] = useState<string | null>(null);
  const [sub, setSub] = useState<string | null>(null);

  useEffect(() => {
    try {
      setTok(localStorage.getItem(KEY));
      setSub(localStorage.getItem(`${KEY}.sub`));
    } catch {
      /* private mode */
    }
  }, []);

  const value = useMemo<AuthState>(
    () => ({
      token,
      sub,
      setToken(t, s) {
        setTok(t);
        setSub(s ?? null);
        try {
          if (t) {
            localStorage.setItem(KEY, t);
            if (s) localStorage.setItem(`${KEY}.sub`, s);
          } else {
            localStorage.removeItem(KEY);
            localStorage.removeItem(`${KEY}.sub`);
          }
        } catch {
          /* ignore */
        }
      },
      signOut() {
        value.setToken(null);
      },
    }),
    [token, sub],
  );

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useAuth(): AuthState {
  const v = useContext(Ctx);
  if (!v) throw new Error('useAuth must be used within AuthProvider');
  return v;
}
