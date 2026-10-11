#!/usr/bin/env node
import { App } from 'aws-cdk-lib';
import { envFromContext, prefix } from '../lib/config.js';
import { AsyncStack } from '../lib/async-stack.js';
import { WebStack } from '../lib/web-stack.js';
import { DataStack } from '../lib/data-stack.js';
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

new DataStack(app, `${prefix(config)}-data`, {
  env,
  config,
  description: 'KabadOS: DynamoDB tables and S3 buckets (PLAN.md §16)',
});

new AsyncStack(app, `${prefix(config)}-async`, {
  env,
  config,
  description: 'KabadOS: EventBridge bus, SQS work queues, Scheduler (PLAN.md §17.3)',
});

new WebStack(app, `${prefix(config)}-web`, {
  env,
  config,
  description: 'KabadOS: static hosting for the client apps (S3 + CloudFront)',
});

app.synth();