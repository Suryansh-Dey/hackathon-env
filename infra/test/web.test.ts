import { App } from 'aws-cdk-lib';
import { Template } from 'aws-cdk-lib/assertions';
import { describe, expect, it } from 'vitest';
import { ENVS } from '../lib/config.js';
import { INDEX_REWRITE_CODE, WebStack } from '../lib/web-stack.js';

describe('WebStack', () => {
  it('creates a private bucket + CloudFront distribution per app', () => {
    const app = new App();
    const stack = new WebStack(app, 'Web', { env: { account: '123456789012', region: 'ap-south-1' }, config: ENVS.dev });
    const t = Template.fromStack(stack);
    t.resourceCountIs('AWS::CloudFront::Distribution', 4);
    t.resourcePropertiesCountIs('AWS::S3::Bucket', { PublicAccessBlockConfiguration: { BlockPublicAcls: true, BlockPublicPolicy: true, IgnorePublicAcls: true, RestrictPublicBuckets: true } }, 4);
    t.resourceCountIs('AWS::CloudFront::Function', 1);
  });

  it('never serves another page for a missing route', () => {
    const app = new App();
    const stack = new WebStack(app, 'Web', { env: { account: '123456789012', region: 'ap-south-1' }, config: ENVS.dev });
    const dists = Template.fromStack(stack).findResources('AWS::CloudFront::Distribution');
    for (const d of Object.values(dists)) {
      const cfg = (d as { Properties: { DistributionConfig: Record<string, any> } }).Properties.DistributionConfig;
      expect(cfg.DefaultCacheBehavior.FunctionAssociations).toHaveLength(1);
      for (const e of cfg.CustomErrorResponses) expect(e).toMatchObject({ ResponseCode: 404, ResponsePagePath: '/404.html' });
    }
  });
});

describe('index rewrite function', () => {
  const handler = new Function(`${INDEX_REWRITE_CODE}; return handler;`)() as (e: { request: { uri: string } }) => { uri: string };
  const rewrite = (uri: string) => handler({ request: { uri } }).uri;

  it('maps route folders to their index.html', () => {
    expect(rewrite('/')).toBe('/index.html');
    expect(rewrite('/signin/')).toBe('/signin/index.html');
    expect(rewrite('/signin')).toBe('/signin/index.html');
    expect(rewrite('/rate-card/')).toBe('/rate-card/index.html');
  });

  it('leaves files untouched', () => {
    expect(rewrite('/_next/static/chunks/3v6e5xwo7dszl.js')).toBe('/_next/static/chunks/3v6e5xwo7dszl.js');
    expect(rewrite('/signin/__next._tree.txt')).toBe('/signin/__next._tree.txt');
    expect(rewrite('/404.html')).toBe('/404.html');
  });
});
