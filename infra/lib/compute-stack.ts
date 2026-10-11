import * as path from 'node:path';
import { fileURLToPath } from 'node:url';
import { CfnOutput, Duration, Stack, StackProps } from 'aws-cdk-lib';
import * as apigw from 'aws-cdk-lib/aws-apigatewayv2';
import * as integrations from 'aws-cdk-lib/aws-apigatewayv2-integrations';
import * as events from 'aws-cdk-lib/aws-events';
import * as targets from 'aws-cdk-lib/aws-events-targets';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as lambda from 'aws-cdk-lib/aws-lambda';
import { SqsEventSource, DynamoEventSource } from 'aws-cdk-lib/aws-lambda-event-sources';
import { RustFunction } from 'cargo-lambda-cdk';
import { Construct } from 'constructs';
import { AsyncStack } from './async-stack.js';
import { EnvConfig, prefix } from './config.js';
import { DataStack } from './data-stack.js';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

export interface ComputeStackProps extends StackProps {
  readonly config: EnvConfig;
  readonly data: DataStack;
  readonly async: AsyncStack;
}

/**
 * Rust Lambdas and the HTTP API (PLAN.md §15.1, §17). The API is one combined
 * Lambda serving the whole axum router behind `$default` (approved packaging
 * choice). Workers deploy as separate Lambdas: a DynamoDB Streams consumer
 * (outbox → EventBridge + notifications), an SQS consumer (media/analysis
 * tasks), an EventBridge consumer (matching + coverage), and a scheduled
 * Rate Index job. Everything is least-privilege.
 */
export class ComputeStack extends Stack {
  readonly api: apigw.HttpApi;

  constructor(scope: Construct, id: string, props: ComputeStackProps) {
    super(scope, id, props);
    const { config, data, async } = props;
    const name = prefix(config);
    const { tables, mediaBucket, documentsBucket, receiptsBucket } = data;

    const commonEnv: Record<string, string> = {
      KABADOS_ENV: config.name,
      TABLE_PREFIX: name,
      MEDIA_BUCKET: mediaBucket.bucketName,
      DOCUMENTS_BUCKET: documentsBucket.bucketName,
      RECEIPTS_BUCKET: receiptsBucket.bucketName,
      EVENT_BUS_NAME: async.bus.eventBusName,
      QUEUE_MEDIA_URL: async.mediaQueue.queueUrl,
      QUEUE_ANALYSIS_URL: async.analysisQueue.queueUrl,
      PUBLIC_BASE_URL: config.name === 'prod' ? 'https://kabados.in' : 'https://dev.kabados.in',
      PUBLIC_SITE_URL: config.sites?.public ?? '',
      KMS_KEY_ID: data.fieldKey.keyArn,
      // Provider modes (external_api.md §21): AI live on Gemini, Razorpay test, rest mocked.
      AI_MODE: 'live',
      PAYMENTS_MODE: 'sandbox',
      WHATSAPP_MODE: 'mock',
      SMS_MODE: 'mock',
      KYC_MODE: 'mock',
      ROUTE_MODE: 'mock',
      PAYOUTS_MODE: 'mock',
      INTEGRITY_MODE: 'mock',
      RUST_LOG: 'info,aws_config=warn',
    };

    const rust = (logicalId: string, binaryName: string, crate: string, opts: Partial<lambda.FunctionOptions> & { memory?: number; timeout?: Duration } = {}) =>
      new RustFunction(this, logicalId, {
        manifestPath: path.join(repoRoot, 'crates', crate, 'Cargo.toml'),
        binaryName,
        architecture: lambda.Architecture.ARM_64,
        memorySize: opts.memory ?? 512,
        timeout: opts.timeout ?? Duration.seconds(30),
        environment: { ...commonEnv, ...(opts.environment ?? {}) },
        bundling: { architecture: lambda.Architecture.ARM_64 },
      });

    // Secrets: the services read /kabados/{env}/{provider}/... by name.
    const secretsArn = `arn:aws:secretsmanager:${this.region}:${this.account}:secret:/kabados/${config.name}/*`;
    const grantSecrets = (fn: lambda.Function) =>
      fn.addToRolePolicy(new iam.PolicyStatement({ actions: ['secretsmanager:GetSecretValue'], resources: [secretsArn] }));

    // Managed AWS calls the services make directly (Location, vision, Chime).
    const grantAwsServices = (fn: lambda.Function) =>
      fn.addToRolePolicy(
        new iam.PolicyStatement({
          actions: [
            'geo-places:Autocomplete', 'geo-places:Geocode', 'geo-places:ReverseGeocode',
            'geo-routes:CalculateRoutes', 'geo-routes:CalculateRouteMatrix',
            'geo:BatchUpdateDevicePosition', 'geo:GetDevicePosition',
            'rekognition:DetectFaces', 'rekognition:DetectModerationLabels', 'rekognition:CompareFaces',
            'textract:AnalyzeDocument', 'textract:DetectDocumentText',
            'chime:CreateMeeting', 'chime:CreateAttendee', 'chime:DeleteMeeting',
            'sns:CreatePlatformEndpoint', 'sns:Publish',
            'ses:SendEmail',
          ],
          resources: ['*'],
        }),
      );

    const grantData = (fn: lambda.Function, write = true) => {
      data.fieldKey.grantEncryptDecrypt(fn);
      for (const t of Object.values(tables)) (write ? t.grantReadWriteData(fn) : t.grantReadData(fn));
      mediaBucket.grantReadWrite(fn);
      documentsBucket.grantReadWrite(fn);
      receiptsBucket.grantReadWrite(fn);
    };

    // ---- combined API Lambda ----
    const apiFn = rust('ApiFn', 'api-all', 'api', { memory: 1024, timeout: Duration.seconds(29) });
    grantData(apiFn);
    grantSecrets(apiFn);
    grantAwsServices(apiFn);
    async.bus.grantPutEventsTo(apiFn);
    async.mediaQueue.grantSendMessages(apiFn);
    async.analysisQueue.grantSendMessages(apiFn);

    this.api = new apigw.HttpApi(this, 'HttpApi', {
      apiName: `${name}-api`,
      corsPreflight: {
        allowHeaders: ['authorization', 'content-type', 'idempotency-key'],
        allowMethods: [apigw.CorsHttpMethod.ANY],
        // Only the KabadOS sites (plus local dev servers outside prod) may call the API from a browser.
        allowOrigins: config.name === 'prod'
          ? ['https://kabados.in']
          : [...Object.values(config.sites ?? {}), 'http://localhost:3001', 'http://localhost:3002', 'http://localhost:3003', 'http://localhost:3004'],
        maxAge: Duration.days(1),
      },
    });
    this.api.addRoutes({
      path: '/{proxy+}',
      methods: [apigw.HttpMethod.ANY],
      integration: new integrations.HttpLambdaIntegration('ApiIntegration', apiFn),
    });

    // ---- workers ----
    const streamFn = rust('StreamWorker', 'worker-stream', 'workers', { timeout: Duration.minutes(1) });
    grantData(streamFn);
    grantSecrets(streamFn);
    grantAwsServices(streamFn);
    async.bus.grantPutEventsTo(streamFn);
    streamFn.addEventSource(
      new DynamoEventSource(tables.core, {
        startingPosition: lambda.StartingPosition.LATEST,
        batchSize: 50,
        retryAttempts: 5,
        bisectBatchOnError: true,
        reportBatchItemFailures: true,
      }),
    );

    const analysisFn = rust('AnalysisWorker', 'worker-analysis', 'workers', { memory: 2048, timeout: Duration.minutes(5) });
    grantData(analysisFn);
    grantSecrets(analysisFn);
    grantAwsServices(analysisFn);
    analysisFn.addEventSource(new SqsEventSource(async.mediaQueue, { batchSize: 1, reportBatchItemFailures: true }));
    analysisFn.addEventSource(new SqsEventSource(async.analysisQueue, { batchSize: 1, reportBatchItemFailures: true }));

    const eventsFn = rust('EventsWorker', 'worker-events', 'workers', { memory: 1024, timeout: Duration.minutes(2) });
    grantData(eventsFn);
    grantSecrets(eventsFn);
    grantAwsServices(eventsFn);
    async.bus.grantPutEventsTo(eventsFn);
    // Matching + coverage: listing, rule-set and authorization events.
    new events.Rule(this, 'MatchingRule', {
      eventBus: async.bus,
      ruleName: `${name}-matching`,
      eventPattern: { source: ['kabados'], detailType: ['ListingPublished', 'ListingVersionCreated', 'ProofProvided', 'RuleSetPublished', 'VendorAuthorizationChanged'] },
      targets: [new targets.LambdaFunction(eventsFn)],
    });

    const rateIndexFn = rust('RateIndexWorker', 'worker-rate-index', 'workers', { timeout: Duration.minutes(5) });
    grantData(rateIndexFn);
    grantSecrets(rateIndexFn);
    // Settlement events feed the running Rate Index / impact aggregates.
    new events.Rule(this, 'SettlementRule', {
      eventBus: async.bus,
      ruleName: `${name}-settlement`,
      eventPattern: { source: ['kabados'], detailType: ['DealSettled'] },
      targets: [new targets.LambdaFunction(rateIndexFn)],
    });
    // Weekly publish pass (Monday 02:00 IST ≈ 20:30 UTC Sunday).
    new events.Rule(this, 'WeeklyRateIndex', {
      ruleName: `${name}-weekly-rate-index`,
      schedule: events.Schedule.cron({ weekDay: 'SUN', hour: '20', minute: '30' }),
      targets: [new targets.LambdaFunction(rateIndexFn, { event: events.RuleTargetInput.fromObject({ job: 'weekly' }) })],
    });

    new CfnOutput(this, 'ApiUrl', { value: this.api.apiEndpoint });
    new CfnOutput(this, 'ApiFunctionName', { value: apiFn.functionName });
  }
}
