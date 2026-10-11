import { App } from 'aws-cdk-lib';
import { Match, Template } from 'aws-cdk-lib/assertions';
import { describe, it } from 'vitest';
import { ENVS } from '../lib/config.js';
import { AsyncStack } from '../lib/async-stack.js';

const synth = () => {
  const app = new App();
  const stack = new AsyncStack(app, 'Async', { env: { account: '123456789012', region: 'ap-south-1' }, config: ENVS.dev });
  return Template.fromStack(stack);
};

describe('AsyncStack', () => {
  it('creates the event bus and a scheduler group', () => {
    const t = synth();
    t.hasResourceProperties('AWS::Events::EventBus', { Name: 'kabados-dev-events' });
    t.hasResourceProperties('AWS::Scheduler::ScheduleGroup', { Name: 'kabados-dev-schedules' });
  });

  it('gives each work queue a DLQ with a redrive policy', () => {
    const t = synth();
    t.resourceCountIs('AWS::SQS::Queue', 4);
    for (const q of ['kabados-dev-media', 'kabados-dev-analysis']) {
      t.hasResourceProperties('AWS::SQS::Queue', { QueueName: q, RedrivePolicy: Match.objectLike({ maxReceiveCount: 5 }) });
    }
  });
});
