import { existsSync } from 'node:fs';
import * as path from 'node:path';
import { fileURLToPath } from 'node:url';
import { CfnOutput, Duration, RemovalPolicy, Stack, StackProps } from 'aws-cdk-lib';
import * as cloudfront from 'aws-cdk-lib/aws-cloudfront';
import * as origins from 'aws-cdk-lib/aws-cloudfront-origins';
import * as s3 from 'aws-cdk-lib/aws-s3';
import * as s3deploy from 'aws-cdk-lib/aws-s3-deployment';
import { Construct } from 'constructs';
import { EnvConfig, prefix } from './config.js';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..');

export interface WebStackProps extends StackProps {
  readonly config: EnvConfig;
}

/**
 * Static hosting for the four client apps (PLAN.md §15.2): each is a private
 * S3 bucket behind a CloudFront distribution with Origin Access Control and an
 * SPA 404→index fallback. The built `web/<app>/out` is uploaded at deploy time;
 * if an app hasn't been built the stack still deploys (empty bucket).
 */
export class WebStack extends Stack {
  readonly urls: Record<string, string> = {};

  constructor(scope: Construct, id: string, props: WebStackProps) {
    super(scope, id, props);
    const name = prefix(props.config);
    const removalPolicy = props.config.hardened ? RemovalPolicy.RETAIN : RemovalPolicy.DESTROY;

    for (const app of ['seller', 'vendor', 'ops', 'public']) {
      const bucket = new s3.Bucket(this, `${app}Bucket`, {
        bucketName: `${name}-web-${app}-${this.account}`,
        blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
        encryption: s3.BucketEncryption.S3_MANAGED,
        enforceSSL: true,
        removalPolicy,
        autoDeleteObjects: !props.config.hardened,
      });

      const distribution = new cloudfront.Distribution(this, `${app}Dist`, {
        comment: `KabadOS ${app} (${props.config.name})`,
        defaultRootObject: 'index.html',
        defaultBehavior: {
          origin: origins.S3BucketOrigin.withOriginAccessControl(bucket),
          viewerProtocolPolicy: cloudfront.ViewerProtocolPolicy.REDIRECT_TO_HTTPS,
          cachePolicy: cloudfront.CachePolicy.CACHING_OPTIMIZED,
        },
        // Static export emits per-route folders; a 403/404 falls back to the SPA shell.
        errorResponses: [
          { httpStatus: 403, responseHttpStatus: 200, responsePagePath: '/index.html', ttl: Duration.minutes(5) },
          { httpStatus: 404, responseHttpStatus: 200, responsePagePath: '/404.html', ttl: Duration.minutes(5) },
        ],
        priceClass: cloudfront.PriceClass.PRICE_CLASS_100,
      });

      const outDir = path.join(repoRoot, 'web', app, 'out');
      new s3deploy.BucketDeployment(this, `${app}Deploy`, {
        destinationBucket: bucket,
        distribution,
        distributionPaths: ['/*'],
        // Falls back to an empty placeholder if the app hasn't been built yet.
        sources: [existsSync(outDir) ? s3deploy.Source.asset(outDir) : s3deploy.Source.data('index.html', `<!doctype html><title>KabadOS ${app}</title><p>Not built yet.</p>`)],
      });

      this.urls[app] = `https://${distribution.distributionDomainName}`;
      new CfnOutput(this, `${app}Url`, { value: this.urls[app] });
    }
  }
}
