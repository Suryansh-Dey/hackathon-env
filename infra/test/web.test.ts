import { App } from 'aws-cdk-lib';
import { Template } from 'aws-cdk-lib/assertions';
import { describe, it } from 'vitest';
import { ENVS } from '../lib/config.js';
import { WebStack } from '../lib/web-stack.js';

describe('WebStack', () => {
  it('creates a private bucket + CloudFront distribution per app', () => {
    const app = new App();
    const stack = new WebStack(app, 'Web', { env: { account: '123456789012', region: 'ap-south-1' }, config: ENVS.dev });
    const t = Template.fromStack(stack);
    t.resourceCountIs('AWS::CloudFront::Distribution', 4);
    t.resourcePropertiesCountIs('AWS::S3::Bucket', { PublicAccessBlockConfiguration: { BlockPublicAcls: true, BlockPublicPolicy: true, IgnorePublicAcls: true, RestrictPublicBuckets: true } }, 4);
  });
});
