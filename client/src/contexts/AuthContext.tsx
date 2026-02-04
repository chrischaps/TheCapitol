import { createContext, useContext, useState, useEffect, type ReactNode } from 'react'
import * as authApi from '../api/auth'

interface AuthState {
  token: string | null
  accountId: string | null
  playerId: string | null
  email: string | null
  tier: string | null
  isAdmin: boolean
}

interface AuthContextType extends AuthState {
  isAuthenticated: boolean
  isLoading: boolean
  login: (email: string, password: string) => Promise<void>
  register: (email: string, password: string, playerName: string) => Promise<void>
  logout: () => Promise<void>
}

const AuthContext = createContext<AuthContextType | null>(null)

const AUTH_STORAGE_KEY = 'capitol_auth'

function loadAuthState(): AuthState {
  try {
    const stored = localStorage.getItem(AUTH_STORAGE_KEY)
    if (stored) {
      return JSON.parse(stored)
    }
  } catch (e) {
    console.error('Failed to load auth state:', e)
  }
  return { token: null, accountId: null, playerId: null, email: null, tier: null, isAdmin: false }
}

function saveAuthState(state: AuthState) {
  try {
    if (state.token) {
      localStorage.setItem(AUTH_STORAGE_KEY, JSON.stringify(state))
    } else {
      localStorage.removeItem(AUTH_STORAGE_KEY)
    }
  } catch (e) {
    console.error('Failed to save auth state:', e)
  }
}

export function AuthProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<AuthState>({ token: null, accountId: null, playerId: null, email: null, tier: null, isAdmin: false })
  const [isLoading, setIsLoading] = useState(true)

  useEffect(() => {
    const stored = loadAuthState()
    setState(stored)
    setIsLoading(false)
  }, [])

  const login = async (email: string, password: string) => {
    const response = await authApi.login(email, password)
    const newState: AuthState = {
      token: response.token,
      accountId: response.account.id,
      playerId: response.player_id,
      email: response.account.email,
      tier: response.account.tier,
      isAdmin: response.account.is_admin,
    }
    setState(newState)
    saveAuthState(newState)
  }

  const register = async (email: string, password: string, playerName: string) => {
    const response = await authApi.register(email, password, playerName)
    const newState: AuthState = {
      token: response.token,
      accountId: response.account.id,
      playerId: response.player_id,
      email: response.account.email,
      tier: response.account.tier,
      isAdmin: response.account.is_admin,
    }
    setState(newState)
    saveAuthState(newState)
  }

  const logout = async () => {
    if (state.token) {
      try {
        await authApi.logout(state.token)
      } catch (e) {
        console.error('Logout API error:', e)
      }
    }
    const emptyState: AuthState = { token: null, accountId: null, playerId: null, email: null, tier: null, isAdmin: false }
    setState(emptyState)
    saveAuthState(emptyState)
  }

  const value: AuthContextType = {
    ...state,
    isAuthenticated: !!state.token,
    isLoading,
    login,
    register,
    logout,
  }

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>
}

export function useAuth() {
  const context = useContext(AuthContext)
  if (!context) {
    throw new Error('useAuth must be used within AuthProvider')
  }
  return context
}
