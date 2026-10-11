import { base } from '@kabados/config/next';
/** @type {import('next').NextConfig} */
export default { ...base, transpilePackages: ['@kabados/ui', '@kabados/api-client'] };
