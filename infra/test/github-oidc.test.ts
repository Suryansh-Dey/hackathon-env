import { App } from 'aws-cdk-lib';
import { Match, Template } from 'aws-cdk-lib/assertions';
import { describe, expect, it } from 'vitest';
import { ENVS } from '../lib/config.js';
import { GithubOidcStack } from '../lib/github-oidc-stack.js';

const template = () => {
  const app = new App();
  const stack = new GithubOidcStack(app, 'Test', { env: { account: '123456789012', region: 'ap-south-1' }, config: ENVS.dev });
  return Template.fromStack(stack);
};

describe('GithubOidcStack', () => {
  it('trusts only this repo on main or the dev environment', () => {
    template().hasResourceProperties('AWS::IAM::Role', {
      RoleName: 'kabados-dev-github-deploy',
      AssumeRolePolicyDocument: Match.objectLike({
        Statement: [
          Match.objectLike({
            Action: 'sts:AssumeRoleWithWebIdentity',
            Condition: {
              StringEquals: { 'token.actions.githubusercontent.com:aud': 'sts.amazonaws.com' },
              StringLike: {
                'token.actions.githubusercontent.com:sub': [
                  'repo:Suryansh-Dey/hackathon-env:ref:refs/heads/main',
                  'repo:Suryansh-Dey/hackathon-env:environment:dev',
                  'repo:Suryansh-Dey@140788315/hackathon-env@1414080614:ref:refs/heads/main',
                  'repo:Suryansh-Dey@140788315/hackathon-env@1414080614:environment:dev',
                ],
              },
            },
          }),
        ],
      }),
    });
  });

  it('can only assume CDK bootstrap roles', () => {
    const t = template();
    const policies = t.findResources('AWS::IAM::Policy', { Properties: { PolicyName: Match.stringLikeRegexp('DeployRole') } });
    const statements = Object.values(policies).flatMap((p) => p.Properties.PolicyDocument.Statement);
    expect(statements).toHaveLength(1);
    expect(statements[0].Action).toBe('sts:AssumeRole');
    expect(JSON.stringify(statements[0].Resource)).toContain('cdk-hnb659fds-');
  });
});
