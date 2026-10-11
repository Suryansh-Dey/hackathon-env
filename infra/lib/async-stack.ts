import { CfnOutput, Duration, Stack, StackProps } from 'aws-cdk-lib';
import * as events from 'aws-cdk-lib/aws-events';
import * as scheduler from 'aws-cdk-lib/aws-scheduler';
import * as sqs from 'aws-cdk-lib/aws-sqs';
import { Construct } from 'constructs';
import { EnvConfig, prefix } from './config.js';

export interface AsyncStackProps extends StackProps {
  readonly config: EnvConfig;
}

/**
 * Eventing backbone (PLAN.md §15.2, §17.3): the domain-event bus, the SQS
 * work queues (media + analysis) each with a dead-letter queue, and a
 * Scheduler group for recurring jobs (Rate Index, listing re-match).
 * Rules and queue consumers are attached in the compute stack, which owns the
 * Lambda functions.
 */
export class AsyncStack extends Stack {
  readonly bus: events.EventBus;
  readonly mediaQueue: sqs.Queue;
  readonly analysisQueue: sqs.Queue;
  readonly schedulerGroup: scheduler.CfnScheduleGroup;

  constructor(scope: Construct, id: string, props: AsyncStackProps) {
    super(scope, id, props);
    const name = prefix(props.config);

    this.bus = new events.EventBus(this, 'Bus', { eventBusName: `${name}-events` });

    const workQueue = (logicalId: string, suffix: string) => {
      const dlq = new sqs.Queue(this, `${logicalId}Dlq`, {
        queueName: `${name}-${suffix}-dlq`,
        retentionPeriod: Duration.days(14),
        enforceSSL: true,
      });
      return new sqs.Queue(this, logicalId, {
        queueName: `${name}-${suffix}`,
        enforceSSL: true,
        // Long enough for Gemini analysis of a full listing (PLAN.md §22.1 p95 < 25s).
        visibilityTimeout: Duration.minutes(6),
        deadLetterQueue: { queue: dlq, maxReceiveCount: 5 },
      });
    };

    this.mediaQueue = workQueue('MediaQueue', 'media');
    this.analysisQueue = workQueue('AnalysisQueue', 'analysis');

    this.schedulerGroup = new scheduler.CfnScheduleGroup(this, 'SchedulerGroup', { name: `${name}-schedules` });

    new CfnOutput(this, 'BusName', { value: this.bus.eventBusName });
    new CfnOutput(this, 'MediaQueueUrl', { value: this.mediaQueue.queueUrl });
    new CfnOutput(this, 'AnalysisQueueUrl', { value: this.analysisQueue.queueUrl });
  }
}
