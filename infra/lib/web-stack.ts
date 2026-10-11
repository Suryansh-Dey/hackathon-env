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
 * Viewer-request rewrite for the Next.js static export (`trailingSlash: true`):
 * S3's REST endpoint behind OAC has no directory index, so `/signin/` must be
 * fetched as `signin/index.html`. Paths with a file extension pass through.
 */
export const INDEX_REWRITE_CODE = `function handler(event) {
  var request = event.request;
  var uri = request.uri;
  if (uri.charAt(uri.length - 1) === '/') {
    request.uri = uri + 'index.html';
  } else if (!/\\.[^\\/]*$/.test(uri)) {
    request.uri = uri + '/index.html';
  }
  return request;
}`;

/**
 * Static hosting for the four client apps (PLAN.md §15.2): each is a private
 * S3 bucket behind a CloudFront distribution with Origin Access Control. Route
 * folders resolve to their `index.html`; anything missing gets the app's 404
 * page. The built `web/<app>/out` is uploaded at deploy time; if an app hasn't
 * been built the stack still deploys (empty bucket).
 */
export class WebStack extends Stack {
  readonly urls: Record<string, string> = {};

  constructor(scope: Construct, id: string, props: WebStackProps) {
    super(scope, id, props);
    const name = prefix(props.config);
    const removalPolicy = props.config.hardened ? RemovalPolicy.RETAIN : RemovalPolicy.DESTROY;

    const indexRewrite = new cloudfront.Function(this, 'IndexRewrite', {
      comment: 'Map /route/ to /route/index.html for the static export',
      runtime: cloudfront.FunctionRuntime.JS_2_0,
      code: cloudfront.FunctionCode.fromInline(INDEX_REWRITE_CODE),
    });

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
          functionAssociations: [{ function: indexRewrite, eventType: cloudfront.FunctionEventType.VIEWER_REQUEST }],
        },
        // OAC grants GetObject only, so a missing key comes back as 403, not 404.
        errorResponses: [
          { httpStatus: 403, responseHttpStatus: 404, responsePagePath: '/404.html', ttl: Duration.minutes(1) },
          { httpStatus: 404, responseHttpStatus: 404, responsePagePath: '/404.html', ttl: Duration.minutes(1) },
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
