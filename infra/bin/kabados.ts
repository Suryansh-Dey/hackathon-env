#!/usr/bin/env node
import { App } from 'aws-cdk-lib';
import { envFromContext, prefix } from '../lib/config.js';
import { GithubOidcStack } from '../lib/github-oidc-stack.js';

const app = new App();
const config = envFromContext(app.node.tryGetContext('env'));
const env = { account: config.account, region: config.region };

// Deployed once by hand; GitHub Actions uses its role to deploy everything else.
new GithubOidcStack(app, `${prefix(config)}-github-oidc`, {
  env,
  config,
  description: 'KabadOS: GitHub Actions OIDC deploy role',
});

app.synth();
