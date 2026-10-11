/** Per-environment settings (PLAN.md §21.1). */
export type EnvName = 'dev' | 'staging' | 'prod';

export interface EnvConfig {
  readonly name: EnvName;
  readonly account: string;
  readonly region: string;
  /** Disaster-recovery region for Global Tables replicas (prod only). */
  readonly drRegion?: string;
  /** Prod-grade protections (deletion protection, retained data, WAF Bot Control). */
  readonly hardened: boolean;
  readonly github: { readonly owner: string; readonly repo: string; readonly branch: string };
}

const ACCOUNT = process.env.CDK_DEFAULT_ACCOUNT ?? '975050124872';

export const ENVS: Record<EnvName, EnvConfig> = {
  dev: {
    name: 'dev',
    account: ACCOUNT,
    region: 'ap-south-1',
    hardened: false,
    github: { owner: 'Suryansh-Dey', repo: 'hackathon-env', branch: 'main' },
  },
  staging: {
    name: 'staging',
    account: ACCOUNT,
    region: 'ap-south-1',
    hardened: true,
    github: { owner: 'Suryansh-Dey', repo: 'hackathon-env', branch: 'main' },
  },
  prod: {
    name: 'prod',
    account: ACCOUNT,
    region: 'ap-south-1',
    drRegion: 'ap-south-2',
    hardened: true,
    github: { owner: 'Suryansh-Dey', repo: 'hackathon-env', branch: 'main' },
  },
};

export function envFromContext(value: unknown): EnvConfig {
  const name = (typeof value === 'string' ? value : 'dev') as EnvName;
  const cfg = ENVS[name];
  if (!cfg) throw new Error(`unknown env ${String(value)}; expected one of ${Object.keys(ENVS).join(', ')}`);
  return cfg;
}

/** Resource name prefix, e.g. `kabados-dev`. */
export const prefix = (cfg: EnvConfig) => `kabados-${cfg.name}`;
