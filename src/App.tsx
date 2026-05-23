import { useState, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { 
  Shield, Zap, ExternalLink, RefreshCw, LogIn, Key, 
  Server, CheckCircle, XCircle, Cpu, Globe, Settings,
  Wifi, WifiOff, LogOut, Square, Activity
} from 'lucide-react';

// ─── Types ────────────────────────────────────────────────────────

interface ModelInfo {
  id: string;
  object?: string;
  owned_by?: string;
}

interface ModelsResponse {
  data: ModelInfo[];
  object?: string;
}

// ─── Constants ────────────────────────────────────────────────────

const LS_TOKEN_KEY = 'ag_token';
const LS_SERVER_KEY = 'ag_server_url';
const LS_IDE_TYPE_KEY = 'ag_ide_type';
const LS_CUSTOM_EXE_KEY = 'ag_custom_exe';
const LS_CUSTOM_DB_KEY = 'ag_custom_db';

const DEFAULT_SERVER_URL = import.meta.env.VITE_SERVER_URL || 'http://127.0.0.1:8045';

// ─── Model Display Helpers ────────────────────────────────────────

const MODEL_CATEGORIES: Record<string, { label: string; color: string; icon: string }> = {
  'gemini':  { label: 'Gemini',  color: 'from-blue-500 to-cyan-400',    icon: '✦' },
  'claude':  { label: 'Claude',  color: 'from-orange-500 to-amber-400', icon: '◈' },
  'gpt':     { label: 'GPT',     color: 'from-emerald-500 to-green-400',icon: '◉' },
};

const IDE_ALLOWED_MODELS = [
  'gemini-3.1-pro-high',
  'gemini-3.1-pro-low',
  'claude-sonnet-4-6-thinking',
  'claude-opus-4-6-thinking',
  'gpt-oss-120b-medium',
  'gemini-3.5-flash-high',
  'gemini-3.5-flash-low'
];

function getModelCategory(modelId: string) {
  for (const [key, val] of Object.entries(MODEL_CATEGORIES)) {
    if (modelId.toLowerCase().includes(key)) return val;
  }
  return { label: 'Other', color: 'from-purple-500 to-pink-400', icon: '◇' };
}

function formatModelName(id: string): string {
  return id
    .replace(/-/g, ' ')
    .replace(/\b\w/g, c => c.toUpperCase());
}

// ─── App Component ────────────────────────────────────────────────

export default function App() {
  const [token, setToken] = useState('');
  const [serverUrl, setServerUrl] = useState(DEFAULT_SERVER_URL);
  
  // Settings State
  const [showSettings, setShowSettings] = useState(false);
  const [ideType, setIdeType] = useState('Antigravity IDE');
  const [customExePath, setCustomExePath] = useState('');
  const [customDbPath, setCustomDbPath] = useState('');

  // Main App State
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [modelPercentages, setModelPercentages] = useState<Record<string, number>>({});
  const [totalCredits, setTotalCredits] = useState<number | null>(null);
  const [creditOverages, setCreditOverages] = useState(false);
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [ideConnecting, setIdeConnecting] = useState(false);
  const [ideSuccess, setIdeSuccess] = useState(false);
  const [serverOnline, setServerOnline] = useState<boolean | null>(null);
  const [proxyRunning, setProxyRunning] = useState(false);

  // ─── Load saved state ────────────────────────────────────────────

  useEffect(() => {
    const savedToken = localStorage.getItem(LS_TOKEN_KEY);
    const savedServer = localStorage.getItem(LS_SERVER_KEY);
    const savedIdeType = localStorage.getItem(LS_IDE_TYPE_KEY);
    const savedExePath = localStorage.getItem(LS_CUSTOM_EXE_KEY);
    const savedDbPath = localStorage.getItem(LS_CUSTOM_DB_KEY);

    if (savedServer) setServerUrl(savedServer);
    if (savedIdeType) setIdeType(savedIdeType);
    if (savedExePath) setCustomExePath(savedExePath);
    if (savedDbPath) setCustomDbPath(savedDbPath);
    
    const url = savedServer || DEFAULT_SERVER_URL;
    
    if (savedToken) {
      setToken(savedToken);
      validateToken(savedToken, url);
    } else {
      checkServerHealth(url);
    }
  }, []);

  // ─── Server Health Check ─────────────────────────────────────────

  const checkServerHealth = useCallback(async (url: string) => {
    try {
      const res = await fetch(`${url}/health`, { 
        method: 'GET',
        signal: AbortSignal.timeout(5000),
      });
      setServerOnline(res.ok);
    } catch {
      setServerOnline(false);
    }
  }, []);

  // ─── Token Validation via /v1/models ─────────────────────────────

  const validateToken = useCallback(async (tokenToUse: string, url?: string) => {
    const serverBase = url || serverUrl;
    if (!tokenToUse) return;
    
    setLoading(true);
    setError('');
    
    try {
      const res = await fetch(`${serverBase}/v1/models`, {
        headers: {
          'Authorization': `Bearer ${tokenToUse}`,
        },
        signal: AbortSignal.timeout(10000),
      });

      if (res.ok) {
        const data: ModelsResponse = await res.json();
        const modelList = (data.data || []).filter(m => IDE_ALLOWED_MODELS.includes(m.id));
        setModels(modelList);
        
        try {
          const qRes = await fetch(`${serverBase}/v1/quota`, {
            headers: { 'Authorization': `Bearer ${tokenToUse}` },
            signal: AbortSignal.timeout(5000),
          });
          if (qRes.ok) {
            const qData = await qRes.json();
            setTotalCredits(qData.total_credits);
            setCreditOverages(qData.enable_credit_overages);
            setModelPercentages(qData.models || {});
          }
        } catch (e) {
          console.error('Failed to fetch quota', e);
        }

        setConnected(true);
        setServerOnline(true);
        localStorage.setItem(LS_TOKEN_KEY, tokenToUse);
        localStorage.setItem(LS_SERVER_KEY, serverBase);
      } else if (res.status === 401) {
        throw new Error('Invalid token. Please check your API key.');
      } else if (res.status === 403) {
        // Manager returns detailed rejection reason in body
        try {
          const body = await res.json();
          const msg = body?.error?.message || 'Access denied';
          throw new Error(msg);
        } catch (e: any) {
          if (e.message && e.message !== 'Access denied') throw e;
          throw new Error('Token rejected (403). It may be expired or IP-limited.');
        }
      } else {
        throw new Error(`Server error (${res.status})`);
      }
    } catch (err: any) {
      if (err.name === 'TimeoutError' || err.name === 'AbortError') {
        setError('Connection timeout. Check server URL and make sure Manager is running.');
      } else {
        setError(err.message || 'Connection error');
      }
      setConnected(false);
      setModels([]);
      checkServerHealth(serverBase);
    } finally {
      setLoading(false);
    }
  }, [serverUrl, checkServerHealth]);

  // ─── Handlers ────────────────────────────────────────────────────

  const handleLogin = (e: React.FormEvent) => {
    e.preventDefault();
    validateToken(token, serverUrl);
  };

  const handleLogout = () => {
    localStorage.removeItem(LS_TOKEN_KEY);
    setToken('');
    setConnected(false);
    setModels([]);
    setError('');
    setIdeSuccess(false);
  };

  const handleRefresh = () => {
    validateToken(token, serverUrl);
  };

  const handleConnect = async () => {
    setIdeConnecting(true);
    setIdeSuccess(false);
    try {
      await invoke('inject_token_and_start_ide', {
        token: token,
        proxyUrl: `${serverUrl}/v1`,
        ideType: ideType,
        customExePath: customExePath || null,
        customDbPath: customDbPath || null,
      });
      setIdeSuccess(true);
      setProxyRunning(true);
      setTimeout(() => setIdeSuccess(false), 5000);
    } catch (err: any) {
      setError(`IDE connection failed: ${err}`);
    } finally {
      setIdeConnecting(false);
    }
  };

  const handleStopProxy = async () => {
    try {
      await invoke('stop_proxy');
      setProxyRunning(false);
    } catch (err: any) {
      setError(`Failed to stop proxy: ${err}`);
    }
  };

  // Poll proxy status
  useEffect(() => {
    const checkProxy = async () => {
      try {
        const running = await invoke<boolean>('get_proxy_status');
        setProxyRunning(running);
      } catch { /* ignore */ }
    };
    checkProxy();
    const interval = setInterval(checkProxy, 3000);
    return () => clearInterval(interval);
  }, []);

  // Poll quota
  useEffect(() => {
    if (!connected || !token) return;
    const fetchQuota = async () => {
      try {
        const qRes = await fetch(`${serverUrl}/v1/quota`, {
          headers: { 'Authorization': `Bearer ${token}` },
          signal: AbortSignal.timeout(5000),
        });
        if (qRes.ok) {
          const qData = await qRes.json();
          setTotalCredits(qData.total_credits);
          setCreditOverages(qData.enable_credit_overages);
          setModelPercentages(qData.models || {});
        }
      } catch (e) {
        // ignore periodic failures
      }
    };
    const interval = setInterval(fetchQuota, 10000);
    return () => clearInterval(interval);
  }, [connected, token, serverUrl]);

  const handleSaveServerUrl = () => {
    localStorage.setItem(LS_SERVER_KEY, serverUrl);
    localStorage.setItem(LS_IDE_TYPE_KEY, ideType);
    localStorage.setItem(LS_CUSTOM_EXE_KEY, customExePath);
    localStorage.setItem(LS_CUSTOM_DB_KEY, customDbPath);
    setShowSettings(false);
    checkServerHealth(serverUrl);
    if (token && connected) {
      validateToken(token, serverUrl);
    }
  };

  const toggleCreditOverages = async () => {
    try {
      const newState = !creditOverages;
      setCreditOverages(newState); // optimistic update
      await fetch(`${serverUrl}/v1/quota/config`, {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
          'Authorization': `Bearer ${token}`
        },
        body: JSON.stringify({ enable_credit_overages: newState }),
      });
    } catch (e) {
      console.error('Failed to toggle config', e);
      setCreditOverages(!creditOverages); // revert on error
    }
  };

  // ─── Render: Login Screen ───────────────────────────────────────

  if (!connected) {
    return (
      <div className="min-h-screen bg-[#0A0A0A] flex items-center justify-center relative overflow-hidden text-white font-sans">
        {/* Background Gradients */}
        <div className="absolute top-[-20%] left-[-10%] w-[50%] h-[50%] bg-blue-600/20 blur-[120px] rounded-full pointer-events-none" />
        <div className="absolute bottom-[-20%] right-[-10%] w-[50%] h-[50%] bg-purple-600/20 blur-[120px] rounded-full pointer-events-none" />

        <div className="relative z-10 w-full max-w-md p-8 backdrop-blur-xl bg-white/5 border border-white/10 shadow-2xl rounded-3xl">
          {/* Logo */}
          <div className="flex flex-col items-center mb-8">
            <div className="w-16 h-16 bg-gradient-to-br from-blue-500 to-purple-600 rounded-2xl flex items-center justify-center mb-4 shadow-lg shadow-purple-500/30">
              <Shield className="w-8 h-8 text-white" />
            </div>
            <h1 className="text-3xl font-bold bg-clip-text text-transparent bg-gradient-to-r from-white to-white/60">
              Antigravity
            </h1>
            <p className="text-sm text-gray-400 mt-2 text-center">
              Secure proxy client
            </p>
          </div>

          <form onSubmit={handleLogin} className="space-y-5">
            {/* Server URL */}
            <div>
              <label className="block text-sm font-medium text-gray-400 mb-2">
                Server URL
              </label>
              <div className="relative">
                <div className="absolute inset-y-0 left-0 pl-4 flex items-center pointer-events-none">
                  <Globe className="h-4 w-4 text-gray-500" />
                </div>
                <input
                  type="text"
                  value={serverUrl}
                  onChange={(e) => setServerUrl(e.target.value.replace(/\/+$/, ''))}
                  className="block w-full pl-10 pr-12 py-3 bg-black/40 border border-white/10 rounded-xl text-white placeholder-gray-500 focus:ring-2 focus:ring-blue-500/50 focus:border-blue-500/50 transition-all outline-none text-sm"
                  placeholder="http://server:8045"
                />
                <div className="absolute inset-y-0 right-0 pr-4 flex items-center">
                  {serverOnline === true && <Wifi className="h-4 w-4 text-emerald-400" />}
                  {serverOnline === false && <WifiOff className="h-4 w-4 text-red-400" />}
                  {serverOnline === null && <div className="h-4 w-4 rounded-full bg-gray-600" />}
                </div>
              </div>
            </div>

            {/* Token Input */}
            <div>
              <label className="block text-sm font-medium text-gray-400 mb-2">
                Access Token
              </label>
              <div className="relative">
                <div className="absolute inset-y-0 left-0 pl-4 flex items-center pointer-events-none">
                  <Key className="h-5 w-5 text-gray-500" />
                </div>
                <input
                  type="password"
                  value={token}
                  onChange={(e) => setToken(e.target.value)}
                  className="block w-full pl-11 pr-4 py-3 bg-black/40 border border-white/10 rounded-xl text-white placeholder-gray-500 focus:ring-2 focus:ring-blue-500/50 focus:border-blue-500/50 transition-all outline-none"
                  placeholder="sk-..."
                  required
                />
              </div>
            </div>

            {/* Error */}
            {error && (
              <div className="text-red-400 text-sm bg-red-400/10 p-3 rounded-lg border border-red-400/20">
                {error}
              </div>
            )}

            {/* Submit */}
            <button
              type="submit"
              disabled={loading || !token}
              className="w-full py-3 px-4 bg-gradient-to-r from-blue-600 to-purple-600 hover:from-blue-500 hover:to-purple-500 text-white rounded-xl font-medium shadow-lg shadow-purple-500/25 transition-all flex items-center justify-center group disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {loading ? (
                <RefreshCw className="w-5 h-5 animate-spin" />
              ) : (
                <>
                  <LogIn className="w-5 h-5 mr-2 group-hover:translate-x-1 transition-transform" />
                  Connect
                </>
              )}
            </button>
          </form>
        </div>
      </div>
    );
  }

  // ─── Render: Dashboard ──────────────────────────────────────────

  // Group models by category
  const groupedModels: Record<string, ModelInfo[]> = {};
  models.forEach(m => {
    const cat = getModelCategory(m.id);
    if (!groupedModels[cat.label]) groupedModels[cat.label] = [];
    groupedModels[cat.label].push(m);
  });

  return (
    <div className="min-h-screen bg-[#0A0A0A] p-6 relative overflow-hidden text-white font-sans flex flex-col items-center">
      <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[80%] h-[500px] bg-blue-600/10 blur-[150px] pointer-events-none" />

      <div className="w-full max-w-4xl relative z-10">
        {/* ─── Header ──────────────────────────────────────────── */}
        <div className="flex justify-between items-center mb-6">
          <div className="flex items-center space-x-3">
            <div className="w-10 h-10 bg-gradient-to-br from-blue-500 to-purple-600 rounded-xl flex items-center justify-center shadow-lg">
              <Shield className="w-5 h-5 text-white" />
            </div>
            <div>
              <h2 className="text-xl font-bold text-white">Antigravity Client</h2>
              <div className="flex items-center space-x-2 text-xs text-gray-400">
                <CheckCircle className="w-3 h-3 text-emerald-400" />
                <span>Connected to {new URL(serverUrl).host}</span>
              </div>
            </div>
          </div>

          <div className="flex items-center space-x-3">
            <button
              onClick={handleRefresh}
              className="p-2 bg-white/5 hover:bg-white/10 rounded-lg transition-colors border border-white/5"
              title="Refresh"
            >
              <RefreshCw className={`w-4 h-4 text-gray-300 ${loading ? 'animate-spin' : ''}`} />
            </button>
            <button
              onClick={() => setShowSettings(!showSettings)}
              className="p-2 bg-white/5 hover:bg-white/10 rounded-lg transition-colors border border-white/5"
              title="Settings"
            >
              <Settings className="w-4 h-4 text-gray-300" />
            </button>
            <button
              onClick={handleLogout}
              className="text-sm px-3 py-2 bg-white/5 hover:bg-red-500/20 hover:text-red-400 rounded-lg transition-colors border border-white/5 flex items-center space-x-1"
            >
              <LogOut className="w-4 h-4" />
              <span>Logout</span>
            </button>
          </div>
        </div>

        {/* ─── Settings Panel ──────────────────────────────────── */}
        {showSettings && (
          <div className="mb-6 backdrop-blur-xl bg-white/5 border border-white/10 rounded-2xl p-5 animate-in fade-in slide-in-from-top-2">
            <h3 className="text-sm font-semibold text-gray-300 mb-3 flex items-center space-x-2">
              <Server className="w-4 h-4" />
              <span>Server Configuration</span>
            </h3>
            <div className="flex space-x-3">
              <input
                type="text"
                value={serverUrl}
                onChange={(e) => setServerUrl(e.target.value.replace(/\/+$/, ''))}
                className="flex-1 px-4 py-2.5 bg-black/40 border border-white/10 rounded-xl text-white text-sm focus:ring-2 focus:ring-blue-500/50 outline-none"
                placeholder="http://server:8045"
              />
              <button
                onClick={handleSaveServerUrl}
                className="px-5 py-2.5 bg-blue-600 hover:bg-blue-500 text-white rounded-xl text-sm font-medium transition-colors"
              >
                Save & Reconnect
              </button>
            </div>
            <div className="mt-3 flex items-center space-x-4 text-xs text-gray-500">
              <span>Proxy URL for IDE: <code className="text-gray-400">{serverUrl}/v1</code></span>
            </div>
          </div>
        )}

        {/* ─── Error Banner ────────────────────────────────────── */}
        {error && (
          <div className="mb-6 text-red-400 text-sm bg-red-400/10 p-4 rounded-xl border border-red-400/20 flex items-start space-x-3">
            <XCircle className="w-5 h-5 flex-shrink-0 mt-0.5" />
            <div>
              <p className="font-medium">Error</p>
              <p className="text-red-300/80 mt-1">{error}</p>
            </div>
          </div>
        )}

        {/* ─── Main Grid ───────────────────────────────────────── */}
        <div className="grid grid-cols-1 md:grid-cols-4 gap-5 mb-6">

          {/* Connection Status Card */}
          <div className="backdrop-blur-xl bg-white/5 border border-white/10 rounded-2xl p-6 relative overflow-hidden group">
            <div className="absolute -bottom-10 -right-10 w-32 h-32 bg-emerald-500/10 blur-[50px] group-hover:bg-emerald-500/20 transition-all" />
            <div className="flex items-center space-x-3 mb-3">
              <div className="p-2 bg-emerald-500/20 rounded-lg border border-emerald-500/30">
                <Wifi className="w-4 h-4 text-emerald-400" />
              </div>
              <h3 className="text-sm font-medium text-gray-300">Status</h3>
            </div>
            <div className="text-2xl font-bold text-emerald-400 mb-1">Active</div>
            <div className="text-xs text-gray-500">Token authenticated</div>
          </div>

          {/* Models Count Card */}
          <div className="backdrop-blur-xl bg-white/5 border border-white/10 rounded-2xl p-6 relative overflow-hidden group">
            <div className="absolute -top-10 -left-10 w-32 h-32 bg-blue-500/10 blur-[50px] group-hover:bg-blue-500/20 transition-all" />
            <div className="flex items-center space-x-3 mb-3">
              <div className="p-2 bg-blue-500/20 rounded-lg border border-blue-500/30">
                <Cpu className="w-4 h-4 text-blue-400" />
              </div>
              <h3 className="text-sm font-medium text-gray-300">Models</h3>
            </div>
            <div className="text-2xl font-bold text-white mb-1">{models.length}</div>
            <div className="text-xs text-gray-500">Available AI models</div>
          </div>

          {/* Credits Card */}
          <div className="backdrop-blur-xl bg-white/5 border border-white/10 rounded-2xl p-6 relative overflow-hidden group flex flex-col justify-between">
            <div className="absolute -bottom-10 -right-10 w-32 h-32 bg-amber-500/10 blur-[50px] group-hover:bg-amber-500/20 transition-all" />
            <div>
              <div className="flex items-center justify-between mb-3">
                <div className="flex items-center space-x-3">
                  <div className="p-2 bg-amber-500/20 rounded-lg border border-amber-500/30">
                    <Zap className="w-4 h-4 text-amber-400" />
                  </div>
                  <h3 className="text-sm font-medium text-gray-300">Credits</h3>
                </div>
                <div className="flex items-center">
                  <label className="relative inline-flex items-center cursor-pointer" title="Enable Credit Overages">
                    <input type="checkbox" className="sr-only peer" checked={creditOverages} onChange={toggleCreditOverages} />
                    <div className="w-9 h-5 bg-gray-600 peer-focus:outline-none rounded-full peer peer-checked:after:translate-x-full peer-checked:after:border-white after:content-[''] after:absolute after:top-[2px] after:left-[2px] after:bg-white after:border-gray-300 after:border after:rounded-full after:h-4 after:w-4 after:transition-all peer-checked:bg-amber-500"></div>
                  </label>
                </div>
              </div>
              <div className="text-2xl font-bold text-amber-400 mb-1">{totalCredits !== null ? totalCredits : '-'}</div>
              <div className="text-xs text-gray-500">Total combined credits</div>
            </div>
          </div>

          {/* Server Card */}
          <div className="backdrop-blur-xl bg-white/5 border border-white/10 rounded-2xl p-6 relative overflow-hidden group">
            <div className="absolute -bottom-10 -left-10 w-32 h-32 bg-purple-500/10 blur-[50px] group-hover:bg-purple-500/20 transition-all" />
            <div className="flex items-center space-x-3 mb-3">
              <div className="p-2 bg-purple-500/20 rounded-lg border border-purple-500/30">
                <Server className="w-4 h-4 text-purple-400" />
              </div>
              <h3 className="text-sm font-medium text-gray-300">Server</h3>
            </div>
            <div className="text-lg font-bold text-white mb-1 truncate" title={new URL(serverUrl).host}>
              {new URL(serverUrl).host}
            </div>
            <div className="text-xs text-gray-500">Proxy endpoint</div>
          </div>
        </div>

        {/* ─── Models List ─────────────────────────────────────── */}
        {models.length > 0 && (
          <div className="backdrop-blur-xl bg-white/5 border border-white/10 rounded-2xl p-6 mb-6">
            <div className="flex items-center justify-between mb-4">
              <div className="flex items-center space-x-3">
                <div className="p-2 bg-blue-500/20 rounded-lg border border-blue-500/30">
                  <Zap className="w-4 h-4 text-blue-400" />
                </div>
                <h3 className="text-base font-semibold text-gray-200">Available Models</h3>
              </div>
              <span className="text-xs text-gray-500">{models.length} models</span>
            </div>

            <div className="space-y-3">
              {Object.entries(groupedModels).map(([category, categoryModels]) => {
                const catInfo = Object.values(MODEL_CATEGORIES).find(c => c.label === category)
                  || { label: category, color: 'from-purple-500 to-pink-400', icon: '◇' };
                
                return (
                  <div key={category}>
                    <div className="flex items-center space-x-2 mb-2">
                      <span className="text-sm">{catInfo.icon}</span>
                      <span className="text-xs font-semibold text-gray-400 uppercase tracking-wider">{category}</span>
                      <span className="text-xs text-gray-600">({categoryModels.length})</span>
                    </div>
                    <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                      {categoryModels.map(model => {
                        const pct = modelPercentages[model.id] ?? modelPercentages[model.id.replace('-thinking', '')];
                        return (
                          <div
                            key={model.id}
                            className="flex flex-col space-y-1.5 px-3 py-2.5 bg-white/[0.03] hover:bg-white/[0.06] border border-white/5 rounded-xl transition-colors group/model"
                          >
                            <div className="flex items-center justify-between">
                              <div className="flex items-center space-x-3">
                                <div className={`w-2 h-2 rounded-full bg-gradient-to-r ${catInfo.color} flex-shrink-0`} />
                                <span className="text-sm text-gray-300 truncate group-hover/model:text-white transition-colors" title={model.id}>
                                  {formatModelName(model.id)}
                                </span>
                              </div>
                              {pct !== undefined && (
                                <span className="text-[10px] text-gray-500 font-mono">{pct}%</span>
                              )}
                            </div>
                            {pct !== undefined && (
                              <div className="w-full bg-white/10 rounded-full h-1 mt-1 overflow-hidden">
                                <div className={`h-1 rounded-full bg-gradient-to-r ${catInfo.color}`} style={{ width: `${Math.min(100, Math.max(0, pct))}%` }}></div>
                              </div>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* ─── Connect to IDE ──────────────────────────────────── */}
        <div className="backdrop-blur-xl bg-white/5 border border-white/10 rounded-2xl p-8 flex flex-col items-center justify-center text-center">
          <div className="w-16 h-16 bg-gradient-to-br from-blue-500/20 to-purple-600/20 rounded-full flex items-center justify-center mb-6 border border-white/10">
            <Shield className="w-8 h-8 text-blue-400" />
          </div>
          <h2 className="text-2xl font-bold text-white mb-2">Ready to Code?</h2>
          <p className="text-gray-400 max-w-lg mb-8">
            Connect to Antigravity IDE with your secure proxy token. A local proxy on port 8046 will route all IDE requests through the Manager.
          </p>

          {/* Proxy Status Badge */}
          {proxyRunning && (
            <div className="mb-6 flex items-center space-x-3">
              <div className="flex items-center space-x-2 text-emerald-400 bg-emerald-400/10 px-4 py-3 rounded-xl border border-emerald-400/20">
                <Activity className="w-4 h-4 animate-pulse" />
                <span className="text-sm font-medium">Local Proxy Active on :8046</span>
              </div>
              <button
                onClick={handleStopProxy}
                className="flex items-center space-x-1 px-3 py-3 bg-red-500/10 hover:bg-red-500/20 text-red-400 rounded-xl border border-red-400/20 transition-colors text-sm"
              >
                <Square className="w-3 h-3" />
                <span>Stop</span>
              </button>
            </div>
          )}

          {ideSuccess && (
            <div className="mb-6 flex items-center space-x-2 text-emerald-400 bg-emerald-400/10 px-4 py-3 rounded-xl border border-emerald-400/20">
              <CheckCircle className="w-5 h-5" />
              <span className="text-sm font-medium">IDE launched successfully! Proxy running.</span>
            </div>
          )}

          {/* IDE Selection */}
          <div className="flex space-x-2 bg-black/40 p-1.5 rounded-xl border border-white/10 mb-6">
            <button
              onClick={() => {
                setIdeType('Antigravity IDE');
                localStorage.setItem(LS_IDE_TYPE_KEY, 'Antigravity IDE');
              }}
              className={`px-4 py-2 rounded-lg text-sm font-medium transition-all ${
                ideType === 'Antigravity IDE' 
                  ? 'bg-blue-500/20 text-blue-400 border border-blue-500/30' 
                  : 'text-gray-400 hover:text-gray-200 border border-transparent'
              }`}
            >
              Antigravity IDE
            </button>
            <button
              onClick={() => {
                setIdeType('Antigravity 2.0');
                localStorage.setItem(LS_IDE_TYPE_KEY, 'Antigravity 2.0');
              }}
              className={`px-4 py-2 rounded-lg text-sm font-medium transition-all ${
                ideType === 'Antigravity 2.0' 
                  ? 'bg-purple-500/20 text-purple-400 border border-purple-500/30' 
                  : 'text-gray-400 hover:text-gray-200 border border-transparent'
              }`}
            >
              Antigravity 2.0
            </button>
          </div>

          <button
            onClick={handleConnect}
            disabled={ideConnecting}
            className="group relative px-8 py-4 bg-white text-black font-semibold rounded-2xl text-lg hover:scale-105 transition-all duration-300 shadow-[0_0_40px_rgba(255,255,255,0.3)] hover:shadow-[0_0_60px_rgba(255,255,255,0.5)] flex items-center space-x-3 overflow-hidden disabled:opacity-50 disabled:cursor-not-allowed disabled:hover:scale-100"
          >
            <div className="absolute inset-0 bg-gradient-to-r from-transparent via-black/5 to-transparent -translate-x-full group-hover:animate-[shimmer_1.5s_infinite]" />
            {ideConnecting ? (
              <RefreshCw className="w-5 h-5 animate-spin" />
            ) : (
              <>
                <span>{proxyRunning ? 'Reconnect IDE' : `Connect to ${ideType}`}</span>
                <ExternalLink className="w-5 h-5" />
              </>
            )}
          </button>
        </div>
      </div>

      {/* ─── Settings Modal ──────────────────────────────────── */}
      {showSettings && (
        <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm">
          <div className="bg-[#1a1c23] border border-white/10 rounded-2xl w-full max-w-lg shadow-2xl flex flex-col">
            <div className="flex items-center justify-between p-6 border-b border-white/10">
              <h2 className="text-xl font-bold text-white flex items-center space-x-2">
                <Settings className="w-5 h-5 text-gray-400" />
                <span>IDE Settings</span>
              </h2>
              <button 
                onClick={() => setShowSettings(false)}
                className="text-gray-400 hover:text-white transition-colors"
              >
                <XCircle className="w-6 h-6" />
              </button>
            </div>
            
            <div className="p-6 space-y-6 overflow-y-auto">
              <div className="space-y-2">
                <label className="text-sm font-medium text-gray-300 block">
                  Custom Executable Path (Optional)
                </label>
                <input
                  type="text"
                  placeholder={`e.g. C:\\Apps\\${ideType}\\Antigravity.exe`}
                  value={customExePath}
                  onChange={(e) => {
                    setCustomExePath(e.target.value);
                    localStorage.setItem(LS_CUSTOM_EXE_KEY, e.target.value);
                  }}
                  className="w-full bg-black/40 border border-white/10 rounded-xl px-4 py-3 text-white placeholder-gray-600 focus:outline-none focus:ring-2 focus:ring-blue-500/50 font-mono text-sm"
                />
                <p className="text-xs text-gray-500">Leave blank to use default OS path.</p>
              </div>

              <div className="space-y-2">
                <label className="text-sm font-medium text-gray-300 block">
                  Custom DB / AppData Path (Optional)
                </label>
                <input
                  type="text"
                  placeholder="e.g. C:\\Data\\User\\globalStorage\\state.vscdb"
                  value={customDbPath}
                  onChange={(e) => {
                    setCustomDbPath(e.target.value);
                    localStorage.setItem(LS_CUSTOM_DB_KEY, e.target.value);
                  }}
                  className="w-full bg-black/40 border border-white/10 rounded-xl px-4 py-3 text-white placeholder-gray-600 focus:outline-none focus:ring-2 focus:ring-blue-500/50 font-mono text-sm"
                />
                <p className="text-xs text-gray-500">Path to state.vscdb for portable installations.</p>
              </div>
            </div>
            
            <div className="p-6 border-t border-white/10 flex justify-end">
              <button
                onClick={() => setShowSettings(false)}
                className="px-6 py-2.5 bg-white text-black font-semibold rounded-xl hover:bg-gray-200 transition-colors"
              >
                Done
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
