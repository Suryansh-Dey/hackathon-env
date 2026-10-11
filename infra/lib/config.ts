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
  /**
   * GitHub repository. The numeric ids match the repo's immutable OIDC subject
   * (`repo:<owner>@<ownerId>/<repo>@<repoId>:…`), which survives renames and can't be reused.
   */
  readonly github: { readonly owner: string; readonly repo: string; readonly branch: string; readonly ownerId: number; readonly repoId: number };
  /** Deployed site origins (CloudFront), used for CORS and absolute links such as receipt verification. */
  readonly sites?: { readonly seller: string; readonly vendor: string; readonly ops: string; readonly public: string };
}

const ACCOUNT = process.env.CDK_DEFAULT_ACCOUNT ?? '975050124872';

export const ENVS: Record<EnvName, EnvConfig> = {
  dev: {
    name: 'dev',
    account: ACCOUNT,
    region: 'ap-south-1',
    hardened: false,
    github: { owner: 'Suryansh-Dey', repo: 'hackathon-env', branch: 'main', ownerId: 140788315, repoId: 1414080614 },
    // From the kabados-dev-web stack outputs.
    sites: {
      seller: 'https://d1kn7g0l0y77vz.cloudfront.net',
      vendor: 'https://dh36dlei05x8v.cloudfront.net',
      ops: 'https://d271ufmc8uwk5y.cloudfront.net',
      public: 'https://d18zzn1na86duz.cloudfront.net',
    },
  },
  staging: {
    name: 'staging',
    account: ACCOUNT,
    region: 'ap-south-1',
    hardened: true,
    github: { owner: 'Suryansh-Dey', repo: 'hackathon-env', branch: 'main', ownerId: 140788315, repoId: 1414080614 },
  },
  prod: {
    name: 'prod',
    account: ACCOUNT,
    region: 'ap-south-1',
    drRegion: 'ap-south-2',
    hardened: true,
    github: { owner: 'Suryansh-Dey', repo: 'hackathon-env', branch: 'main', ownerId: 140788315, repoId: 1414080614 },
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
