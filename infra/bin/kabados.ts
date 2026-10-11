#!/usr/bin/env node
import { App } from 'aws-cdk-lib';
import { envFromContext, prefix } from '../lib/config.js';
import { PipelineStack } from '../lib/pipeline-stack.js';

const app = new App();
const config = envFromContext(app.node.tryGetContext('env'));
const env = { account: config.account, region: config.region };

new PipelineStack(app, `${prefix(config)}-pipeline`, {
  env,
  config,
  prChecks: app.node.tryGetContext('prChecks') === 'true',
  description: 'KabadOS CI/CD: GitHub → CodePipeline → CodeBuild (PLAN.md §21.3)',
});

app.synth();
