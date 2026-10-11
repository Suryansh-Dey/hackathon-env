import { CfnOutput, Duration, Stack, StackProps } from 'aws-cdk-lib';
import * as iam from 'aws-cdk-lib/aws-iam';
import { Construct } from 'constructs';
import { EnvConfig, prefix } from './config.js';

export interface GithubOidcStackProps extends StackProps {
  readonly config: EnvConfig;
}

/**
 * Lets GitHub Actions deploy with short-lived credentials via OIDC (no
 * long-lived AWS keys). The role may only assume the CDK bootstrap roles,
 * so what it can change is exactly what `cdk deploy` can change.
 */
export class GithubOidcStack extends Stack {
  readonly deployRole: iam.Role;

  constructor(scope: Construct, id: string, props: GithubOidcStackProps) {
    super(scope, id, props);
    const { owner, repo, branch } = props.config.github;

    const provider = new iam.OpenIdConnectProvider(this, 'GitHubOidc', {
      url: 'https://token.actions.githubusercontent.com',
      clientIds: ['sts.amazonaws.com'],
    });

    this.deployRole = new iam.Role(this, 'DeployRole', {
      roleName: `${prefix(props.config)}-github-deploy`,
      description: `GitHub Actions deploys for ${owner}/${repo}@${branch}`,
      maxSessionDuration: Duration.hours(2),
      assumedBy: new iam.WebIdentityPrincipal(provider.openIdConnectProviderArn, {
        StringEquals: { 'token.actions.githubusercontent.com:aud': 'sts.amazonaws.com' },
        StringLike: {
          'token.actions.githubusercontent.com:sub': [
            `repo:${owner}/${repo}:ref:refs/heads/${branch}`,
            `repo:${owner}/${repo}:environment:${props.config.name}`,
          ],
        },
      }),
    });
    this.deployRole.addToPolicy(
      new iam.PolicyStatement({
        actions: ['sts:AssumeRole'],
        resources: [`arn:aws:iam::${this.account}:role/cdk-hnb659fds-*-${this.account}-${this.region}`],
      }),
    );

    new CfnOutput(this, 'DeployRoleArn', { value: this.deployRole.roleArn });
  }
}
