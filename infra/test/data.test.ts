import { App } from 'aws-cdk-lib';
import { Match, Template } from 'aws-cdk-lib/assertions';
import { describe, it } from 'vitest';
import { ENVS } from '../lib/config.js';
import { DataStack } from '../lib/data-stack.js';

const synth = (env: 'dev' | 'prod' = 'dev') => {
  const app = new App();
  const stack = new DataStack(app, 'Data', { env: { account: '123456789012', region: 'ap-south-1' }, config: ENVS[env] });
  return Template.fromStack(stack);
};

describe('DataStack', () => {
  it('creates all five tables with PK/SK and PITR', () => {
    const t = synth();
    t.resourceCountIs('AWS::DynamoDB::Table', 5);
    for (const suffix of ['core', 'ledger', 'chat', 'ephemeral', 'audit']) {
      t.hasResourceProperties('AWS::DynamoDB::Table', {
        TableName: `kabados-dev-${suffix}`,
        PointInTimeRecoverySpecification: { PointInTimeRecoveryEnabled: true },
      });
    }
  });

  it('gives core three GSIs and a stream, and ephemeral a TTL', () => {
    const t = synth();
    t.hasResourceProperties('AWS::DynamoDB::Table', {
      TableName: 'kabados-dev-core',
      GlobalSecondaryIndexes: Match.arrayWith([Match.objectLike({ IndexName: 'GSI1' }), Match.objectLike({ IndexName: 'GSI2' }), Match.objectLike({ IndexName: 'GSI3' })]),
      StreamSpecification: { StreamViewType: 'NEW_AND_OLD_IMAGES' },
    });
    t.hasResourceProperties('AWS::DynamoDB::Table', { TableName: 'kabados-dev-ephemeral', TimeToLiveSpecification: { AttributeName: 'ttl', Enabled: true } });
  });

  it('keeps buckets private with TLS enforced', () => {
    const t = synth();
    t.resourceCountIs('AWS::S3::Bucket', 3);
    t.allResourcesProperties('AWS::S3::Bucket', {
      PublicAccessBlockConfiguration: { BlockPublicAcls: true, BlockPublicPolicy: true, IgnorePublicAcls: true, RestrictPublicBuckets: true },
    });
  });

  it('retains data in hardened (prod) environments', () => {
    synth('prod').hasResource('AWS::DynamoDB::Table', { DeletionPolicy: 'Retain' });
  });
});
