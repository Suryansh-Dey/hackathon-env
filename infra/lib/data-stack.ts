import { CfnOutput, Duration, RemovalPolicy, Stack, StackProps } from 'aws-cdk-lib';
import * as dynamodb from 'aws-cdk-lib/aws-dynamodb';
import * as kms from 'aws-cdk-lib/aws-kms';
import * as s3 from 'aws-cdk-lib/aws-s3';
import { Construct } from 'constructs';
import { EnvConfig, prefix } from './config.js';

export interface DataStackProps extends StackProps {
  readonly config: EnvConfig;
}

/**
 * DynamoDB tables and S3 buckets (PLAN.md §16, §15.2). The table shapes mirror
 * `kabados-db` exactly: generic PK/SK, GSI1-3 on `core`, Streams on the tables
 * that drive the outbox/analytics, TTL on `ephemeral`, PITR and KMS everywhere.
 */
export class DataStack extends Stack {
  readonly tables: Record<string, dynamodb.Table> = {};
  readonly mediaBucket: s3.Bucket;
  readonly documentsBucket: s3.Bucket;
  readonly receiptsBucket: s3.Bucket;
  /** Envelope key for field-level encryption of phones, addresses and documents (PLAN.md §20.1). */
  readonly fieldKey: kms.Key;

  constructor(scope: Construct, id: string, props: DataStackProps) {
    super(scope, id, props);
    const { config } = props;
    const name = prefix(config);
    const removalPolicy = config.hardened ? RemovalPolicy.RETAIN : RemovalPolicy.DESTROY;

    this.fieldKey = new kms.Key(this, 'FieldKey', {
      alias: `${name}-fields`,
      description: 'KabadOS field-level encryption (data keys per container)',
      enableKeyRotation: true,
      removalPolicy,
    });

    const str = (n: string): dynamodb.Attribute => ({ name: n, type: dynamodb.AttributeType.STRING });

    const makeTable = (suffix: string, opts: { gsis?: boolean; stream?: boolean; ttl?: boolean }) => {
      const table = new dynamodb.Table(this, `Table-${suffix}`, {
        tableName: `${name}-${suffix}`,
        partitionKey: str('PK'),
        sortKey: str('SK'),
        billingMode: dynamodb.BillingMode.PAY_PER_REQUEST,
        encryption: dynamodb.TableEncryption.AWS_MANAGED,
        pointInTimeRecoverySpecification: { pointInTimeRecoveryEnabled: true },
        timeToLiveAttribute: opts.ttl ? 'ttl' : undefined,
        stream: opts.stream ? dynamodb.StreamViewType.NEW_AND_OLD_IMAGES : undefined,
        removalPolicy,
        deletionProtection: config.hardened,
      });
      if (opts.gsis) {
        for (const n of [1, 2, 3]) {
          table.addGlobalSecondaryIndex({
            indexName: `GSI${n}`,
            partitionKey: str(`GSI${n}PK`),
            sortKey: str(`GSI${n}SK`),
            projectionType: dynamodb.ProjectionType.ALL,
          });
        }
      }
      this.tables[suffix] = table;
      return table;
    };

    // Matches kabados_db::store::Table: core has GSIs; core/ledger/audit stream; core (outbox,
    // notify, job offers) and ephemeral expire via TTL.
    makeTable('core', { gsis: true, stream: true, ttl: true });
    makeTable('ledger', { stream: true });
    makeTable('chat', {});
    makeTable('ephemeral', { ttl: true });
    makeTable('audit', { stream: true });

    const bucket = (logicalId: string, suffix: string, extra?: Partial<s3.BucketProps>) =>
      new s3.Bucket(this, logicalId, {
        bucketName: `${name}-${suffix}-${this.account}`,
        encryption: s3.BucketEncryption.S3_MANAGED,
        blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
        enforceSSL: true,
        versioned: config.hardened,
        removalPolicy,
        autoDeleteObjects: !config.hardened,
        cors: [
          {
            allowedMethods: [s3.HttpMethods.PUT, s3.HttpMethods.GET, s3.HttpMethods.HEAD],
            allowedOrigins: ['*'],
            allowedHeaders: ['*'],
            maxAge: 3000,
          },
        ],
        ...extra,
      });

    // Listing photos: originals kept briefly, processed copies served to vendors.
    this.mediaBucket = bucket('MediaBucket', 'media', {
      lifecycleRules: [{ prefix: 'listings/', expiration: Duration.days(config.hardened ? 180 : 30) }],
    });
    this.documentsBucket = bucket('DocumentsBucket', 'documents');
    this.receiptsBucket = bucket('ReceiptsBucket', 'receipts', { versioned: true });

    new CfnOutput(this, 'CoreTableName', { value: this.tables.core.tableName });
    new CfnOutput(this, 'CoreStreamArn', { value: this.tables.core.tableStreamArn ?? '' });
    new CfnOutput(this, 'MediaBucketName', { value: this.mediaBucket.bucketName });
  }
}
