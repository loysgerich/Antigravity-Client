import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Shield, Zap, Clock, ExternalLink, RefreshCw, LogIn, HardDrive, Key } from 'lucide-react';

interface TokenStatus {
  id: string;
  username: string;
  expires_type: string;
  expires_at: number | null;
  total_requests: number;
  total_tokens_used: number;
  allocated_accounts_count: number;
  quota_limit: number;
  percentage_used: number;
}

export default function App() {
  const [token, setToken] = useState('');
  const [status, setStatus] = useState<TokenStatus | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  // Адрес сервера берётся из .env (VITE_SERVER_URL)
  // Для продакшена замените на публичный домен в .env файле
  const SERVER_URL = import.meta.env.VITE_SERVER_URL || 'http://127.0.0.1:8046';
  const API_URL = `${SERVER_URL}/v1/token/status`;

  useEffect(() => {
    const savedToken = localStorage.getItem('ag_token');
    if (savedToken) {
      setToken(savedToken);
      fetchStatus(savedToken);
    }
  }, []);

  const fetchStatus = async (tokenToUse: string) => {
    if (!tokenToUse) return;
    setLoading(true);
    setError('');
    try {
      // Имитация задержки для анимации
      await new Promise(r => setTimeout(r, 600));

      const res = await fetch(API_URL, {
        headers: {
          'Authorization': `Bearer ${tokenToUse}`
        }
      });
      if (!res.ok) throw new Error('Invalid or expired token');
      const data = await res.json();
      setStatus(data);
      localStorage.setItem('ag_token', tokenToUse);
    } catch (err: any) {
      setError(err.message || 'Connection error');
      setStatus(null);
    } finally {
      setLoading(false);
    }
  };

  const handleLogin = (e: React.FormEvent) => {
    e.preventDefault();
    fetchStatus(token);
  };

  const handleLogout = () => {
    localStorage.removeItem('ag_token');
    setToken('');
    setStatus(null);
  };

  const calculateDaysLeft = (expiresAt: number | null) => {
    if (!expiresAt) return 'Unlimited';
    const now = Math.floor(Date.now() / 1000);
    const diff = expiresAt - now;
    if (diff <= 0) return 'Expired';
    return Math.ceil(diff / (60 * 60 * 24)) + ' days';
  };

  const handleConnect = async () => {
    try {
      await invoke('inject_token_and_start_ide', { 
        token: token,
        proxyUrl: `${SERVER_URL}/v1`
      });
      // Optionally show a temporary success state
    } catch (err: any) {
      alert(`Failed to connect: ${err}`);
    }
  };

  if (!status) {
    return (
      <div className="min-h-screen bg-[#0A0A0A] flex items-center justify-center relative overflow-hidden text-white font-sans">
        {/* Background Gradients */}
        <div className="absolute top-[-20%] left-[-10%] w-[50%] h-[50%] bg-blue-600/20 blur-[120px] rounded-full pointer-events-none" />
        <div className="absolute bottom-[-20%] right-[-10%] w-[50%] h-[50%] bg-purple-600/20 blur-[120px] rounded-full pointer-events-none" />
        
        <div className="relative z-10 w-full max-w-md p-8 backdrop-blur-xl bg-white/5 border border-white/10 shadow-2xl rounded-3xl">
          <div className="flex flex-col items-center mb-8">
            <div className="w-16 h-16 bg-gradient-to-br from-blue-500 to-purple-600 rounded-2xl flex items-center justify-center mb-4 shadow-lg shadow-purple-500/30">
              <Shield className="w-8 h-8 text-white" />
            </div>
            <h1 className="text-3xl font-bold bg-clip-text text-transparent bg-gradient-to-r from-white to-white/60">
              Antigravity
            </h1>
            <p className="text-sm text-gray-400 mt-2 text-center">
              Secure client interface
            </p>
          </div>

          <form onSubmit={handleLogin} className="space-y-6">
            <div>
              <label className="block text-sm font-medium text-gray-400 mb-2">Access Token</label>
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

            {error && (
              <div className="text-red-400 text-sm bg-red-400/10 p-3 rounded-lg border border-red-400/20">
                {error}
              </div>
            )}

            <button
              type="submit"
              disabled={loading}
              className="w-full py-3 px-4 bg-gradient-to-r from-blue-600 to-purple-600 hover:from-blue-500 hover:to-purple-500 text-white rounded-xl font-medium shadow-lg shadow-purple-500/25 transition-all flex items-center justify-center group disabled:opacity-50 disabled:cursor-not-allowed"
            >
              {loading ? (
                <RefreshCw className="w-5 h-5 animate-spin" />
              ) : (
                <>
                  <LogIn className="w-5 h-5 mr-2 group-hover:translate-x-1 transition-transform" />
                  Activate Token
                </>
              )}
            </button>
          </form>
        </div>
      </div>
    );
  }

  const daysLeft = calculateDaysLeft(status.expires_at);
  const isUnlimited = status.quota_limit === 0;

  return (
    <div className="min-h-screen bg-[#0A0A0A] p-8 relative overflow-hidden text-white font-sans flex flex-col items-center">
      <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[80%] h-[500px] bg-blue-600/10 blur-[150px] pointer-events-none" />
      
      <div className="w-full max-w-4xl relative z-10">
        {/* Header */}
        <div className="flex justify-between items-center mb-8">
          <div className="flex items-center space-x-3">
            <div className="w-10 h-10 bg-gradient-to-br from-blue-500 to-purple-600 rounded-xl flex items-center justify-center shadow-lg">
              <Shield className="w-5 h-5 text-white" />
            </div>
            <div>
              <h2 className="text-xl font-bold text-white">Hello, {status.username}</h2>
              <p className="text-xs text-gray-400">Token ID: {status.id.substring(0, 8)}...</p>
            </div>
          </div>
          <div className="flex items-center space-x-4">
            <button 
              onClick={() => fetchStatus(token)}
              className="p-2 bg-white/5 hover:bg-white/10 rounded-lg transition-colors border border-white/5"
              title="Refresh Stats"
            >
              <RefreshCw className={`w-5 h-5 text-gray-300 ${loading ? 'animate-spin' : ''}`} />
            </button>
            <button 
              onClick={handleLogout}
              className="text-sm px-4 py-2 bg-white/5 hover:bg-red-500/20 hover:text-red-400 rounded-lg transition-colors border border-white/5"
            >
              Log out
            </button>
          </div>
        </div>

        {/* Main Stats Grid */}
        <div className="grid grid-cols-1 md:grid-cols-3 gap-6 mb-8">
          
          {/* Quota Card */}
          <div className="col-span-1 md:col-span-2 backdrop-blur-xl bg-white/5 border border-white/10 rounded-3xl p-8 relative overflow-hidden group">
            <div className="absolute top-0 right-0 w-64 h-64 bg-blue-500/10 blur-[80px] group-hover:bg-blue-500/20 transition-all" />
            
            <div className="flex items-center space-x-3 mb-6">
              <div className="p-2 bg-blue-500/20 rounded-lg border border-blue-500/30">
                <Zap className="w-5 h-5 text-blue-400" />
              </div>
              <h3 className="text-lg font-semibold text-gray-200">Token Quota</h3>
            </div>

            <div className="flex flex-col space-y-4">
              <div className="flex justify-between items-end">
                <div>
                  <div className="text-4xl font-bold text-white mb-1">
                    {isUnlimited ? 'Unlimited' : `${status.percentage_used}%`}
                  </div>
                  <div className="text-sm text-gray-400">
                    {status.total_tokens_used.toLocaleString()} / {isUnlimited ? '∞' : status.quota_limit.toLocaleString()} tokens used
                  </div>
                </div>
              </div>

              {!isUnlimited && (
                <div className="w-full h-3 bg-black/50 rounded-full overflow-hidden border border-white/5">
                  <div 
                    className="h-full bg-gradient-to-r from-blue-500 to-purple-500 rounded-full transition-all duration-1000 ease-out relative"
                    style={{ width: `${Math.min(status.percentage_used, 100)}%` }}
                  >
                    <div className="absolute top-0 right-0 bottom-0 left-0 bg-[url('data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciIHdpZHRoPSI4IiBoZWlnaHQ9IjgiPgo8cmVjdCB3aWR0aD0iOCIgaGVpZ2h0PSI4IiBmaWxsPSIjZmZmIiBmaWxsLW9wYWNpdHk9IjAuMSIvPgo8L3N2Zz4=')] opacity-20" />
                  </div>
                </div>
              )}
            </div>
          </div>

          {/* Time & Account Cards */}
          <div className="flex flex-col gap-6">
            <div className="flex-1 backdrop-blur-xl bg-white/5 border border-white/10 rounded-3xl p-6 flex flex-col justify-center relative overflow-hidden group">
              <div className="absolute -bottom-10 -right-10 w-32 h-32 bg-purple-500/20 blur-[50px] group-hover:bg-purple-500/30 transition-all" />
              <div className="flex items-center space-x-3 mb-2">
                <Clock className="w-5 h-5 text-purple-400" />
                <h3 className="text-sm font-medium text-gray-300">Time Remaining</h3>
              </div>
              <div className="text-3xl font-bold text-white">{daysLeft}</div>
              <div className="text-xs text-gray-500 mt-1 capitalize">{status.expires_type} access</div>
            </div>

            <div className="flex-1 backdrop-blur-xl bg-white/5 border border-white/10 rounded-3xl p-6 flex flex-col justify-center relative overflow-hidden group">
               <div className="absolute -top-10 -left-10 w-32 h-32 bg-emerald-500/10 blur-[50px] group-hover:bg-emerald-500/20 transition-all" />
              <div className="flex items-center space-x-3 mb-2">
                <HardDrive className="w-5 h-5 text-emerald-400" />
                <h3 className="text-sm font-medium text-gray-300">Bound Accounts</h3>
              </div>
              <div className="text-3xl font-bold text-white">
                {status.allocated_accounts_count === 0 ? 'Shared Pool' : status.allocated_accounts_count}
              </div>
              <div className="text-xs text-gray-500 mt-1">Dedicated proxy nodes</div>
            </div>
          </div>
        </div>

        {/* Action Area */}
        <div className="backdrop-blur-xl bg-white/5 border border-white/10 rounded-3xl p-8 flex flex-col items-center justify-center text-center">
          <div className="w-16 h-16 bg-gradient-to-br from-blue-500/20 to-purple-600/20 rounded-full flex items-center justify-center mb-6 border border-white/10">
            <Shield className="w-8 h-8 text-blue-400" />
          </div>
          <h2 className="text-2xl font-bold text-white mb-2">Ready to Code?</h2>
          <p className="text-gray-400 max-w-lg mb-8">
            Click the button below to automatically configure and launch Antigravity IDE with your secure token. No manual setup required.
          </p>
          <button 
            onClick={handleConnect}
            className="group relative px-8 py-4 bg-white text-black font-semibold rounded-2xl text-lg hover:scale-105 transition-all duration-300 shadow-[0_0_40px_rgba(255,255,255,0.3)] hover:shadow-[0_0_60px_rgba(255,255,255,0.5)] flex items-center space-x-3 overflow-hidden"
          >
            <div className="absolute inset-0 bg-gradient-to-r from-transparent via-black/5 to-transparent -translate-x-full group-hover:animate-[shimmer_1.5s_infinite]" />
            <span>Connect to Antigravity IDE</span>
            <ExternalLink className="w-5 h-5" />
          </button>
        </div>

      </div>
    </div>
  );
}
