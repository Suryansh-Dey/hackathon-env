import { App } from 'aws-cdk-lib';
import { Match, Template } from 'aws-cdk-lib/assertions';
import { describe, expect, it } from 'vitest';
import { ENVS } from '../lib/config.js';
import { PipelineStack } from '../lib/pipeline-stack.js';

const synth = (prChecks = false) => {
  const app = new App();
  const config = ENVS.dev;
  const stack = new PipelineStack(app, 'Test', { env: { account: '123456789012', region: 'ap-south-1' }, config, prChecks });
  return Template.fromStack(stack);
};

describe('PipelineStack', () => {
  it('creates a GitHub CodeConnections connection', () => {
    synth().hasResourceProperties('AWS::CodeStarConnections::Connection', { ProviderType: 'GitHub' });
  });

  it('runs backend CI on a privileged arm64 CodeBuild image', () => {
    synth().hasResourceProperties('AWS::CodeBuild::Project', {
      Name: 'kabados-dev-backend-ci',
      Environment: Match.objectLike({ Type: 'ARM_CONTAINER', PrivilegedMode: true }),
      Source: Match.objectLike({ BuildSpec: 'ci/buildspec-backend.yml' }),
    });
  });

  it('triggers a V2 pipeline on pushes to main', () => {
    const t = synth();
    t.hasResourceProperties('AWS::CodePipeline::Pipeline', {
      PipelineType: 'V2',
      Triggers: [
        Match.objectLike({
          GitConfiguration: Match.objectLike({ Push: [Match.objectLike({ Branches: { Includes: ['main'] } })] }),
        }),
      ],
    });
    const stages = Object.values(t.findResources('AWS::CodePipeline::Pipeline'))[0].Properties.Stages;
    expect(stages.map((s: { Name: string }) => s.Name)).toEqual(['Source', 'CI']);
  });

  it('adds PR status-check webhooks only when enabled', () => {
    synth(false).resourceCountIs('AWS::CodeBuild::SourceCredential', 0);
    const t = synth(true);
    t.resourceCountIs('AWS::CodeBuild::SourceCredential', 1);
    t.hasResourceProperties('AWS::CodeBuild::Project', {
      Name: 'kabados-dev-backend-pr',
      Triggers: Match.objectLike({ Webhook: true }),
    });
  });

  it('keeps build buckets private', () => {
    synth().allResourcesProperties('AWS::S3::Bucket', {
      PublicAccessBlockConfiguration: { BlockPublicAcls: true, BlockPublicPolicy: true, IgnorePublicAcls: true, RestrictPublicBuckets: true },
    });
  });
});
