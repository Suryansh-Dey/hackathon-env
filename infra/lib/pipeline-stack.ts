import { Duration, RemovalPolicy, Stack, StackProps } from 'aws-cdk-lib';
import * as codebuild from 'aws-cdk-lib/aws-codebuild';
import * as codepipeline from 'aws-cdk-lib/aws-codepipeline';
import * as actions from 'aws-cdk-lib/aws-codepipeline-actions';
import * as connections from 'aws-cdk-lib/aws-codestarconnections';
import * as iam from 'aws-cdk-lib/aws-iam';
import * as s3 from 'aws-cdk-lib/aws-s3';
import { Construct } from 'constructs';
import { EnvConfig, prefix } from './config.js';

export interface PipelineStackProps extends StackProps {
  readonly config: EnvConfig;
  /**
   * Adds CodeBuild webhook projects that report status checks on GitHub pull
   * requests. Requires the CodeConnections connection to be AVAILABLE, so it
   * is enabled with `-c prChecks=true` after the one-time console handshake.
   */
  readonly prChecks?: boolean;
}

interface CiJob {
  readonly id: string;
  readonly buildspec: string;
  readonly arm: boolean;
  readonly compute: codebuild.ComputeType;
  readonly privileged?: boolean;
  readonly timeout: Duration;
}

/** CI/CD (PLAN.md §21.3): GitHub via CodeConnections → CodePipeline V2 → CodeBuild. */
export class PipelineStack extends Stack {
  readonly connectionArn: string;
  readonly pipeline: codepipeline.Pipeline;

  constructor(scope: Construct, id: string, props: PipelineStackProps) {
    super(scope, id, props);
    const { config } = props;
    const name = prefix(config);

    const connection = new connections.CfnConnection(this, 'GitHubConnection', {
      connectionName: `${name}-github`,
      providerType: 'GitHub',
    });
    this.connectionArn = connection.attrConnectionArn;

    const artifacts = new s3.Bucket(this, 'Artifacts', {
      encryption: s3.BucketEncryption.S3_MANAGED,
      blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
      enforceSSL: true,
      lifecycleRules: [{ expiration: Duration.days(30) }],
      removalPolicy: RemovalPolicy.DESTROY,
      autoDeleteObjects: true,
    });
    const cache = new s3.Bucket(this, 'BuildCache', {
      encryption: s3.BucketEncryption.S3_MANAGED,
      blockPublicAccess: s3.BlockPublicAccess.BLOCK_ALL,
      enforceSSL: true,
      lifecycleRules: [{ expiration: Duration.days(14) }],
      removalPolicy: RemovalPolicy.DESTROY,
      autoDeleteObjects: true,
    });

    const jobs: CiJob[] = [
      {
        id: 'Backend',
        buildspec: 'ci/buildspec-backend.yml',
        arm: true,
        compute: codebuild.ComputeType.LARGE,
        privileged: true,
        timeout: Duration.minutes(90),
      },
      { id: 'Infra', buildspec: 'ci/buildspec-infra.yml', arm: false, compute: codebuild.ComputeType.SMALL, timeout: Duration.minutes(20) },
    ];

    const environmentFor = (job: CiJob): codebuild.BuildEnvironment => ({
      buildImage: job.arm
        ? codebuild.LinuxArmBuildImage.AMAZON_LINUX_2023_STANDARD_3_0
        : codebuild.LinuxBuildImage.AMAZON_LINUX_2023_5,
      computeType: job.compute,
      privileged: job.privileged ?? false,
    });

    const projects = jobs.map((job) => {
      const project = new codebuild.PipelineProject(this, `${job.id}Ci`, {
        projectName: `${name}-${job.id.toLowerCase()}-ci`,
        buildSpec: codebuild.BuildSpec.fromSourceFilename(job.buildspec),
        environment: environmentFor(job),
        cache: codebuild.Cache.bucket(cache, { prefix: job.id.toLowerCase() }),
        timeout: job.timeout,
      });
      return { job, project };
    });
    // Infra CI runs `cdk synth`, which looks up the account context.
    projects
      .find((p) => p.job.id === 'Infra')
      ?.project.addToRolePolicy(
        new iam.PolicyStatement({ actions: ['sts:AssumeRole'], resources: [`arn:aws:iam::${this.account}:role/cdk-*-lookup-role-*`] }),
      );

    const sourceOutput = new codepipeline.Artifact('Source');
    const source = new actions.CodeStarConnectionsSourceAction({
      actionName: 'GitHub',
      connectionArn: connection.attrConnectionArn,
      owner: config.github.owner,
      repo: config.github.repo,
      branch: config.github.branch,
      output: sourceOutput,
      codeBuildCloneOutput: true,
      triggerOnPush: true,
    });

    this.pipeline = new codepipeline.Pipeline(this, 'Pipeline', {
      pipelineName: `${name}-pipeline`,
      pipelineType: codepipeline.PipelineType.V2,
      executionMode: codepipeline.ExecutionMode.QUEUED,
      artifactBucket: artifacts,
      triggers: [
        {
          providerType: codepipeline.ProviderType.CODE_STAR_SOURCE_CONNECTION,
          gitConfiguration: {
            sourceAction: source,
            pushFilter: [{ branchesIncludes: [config.github.branch] }],
          },
        },
      ],
      stages: [
        { stageName: 'Source', actions: [source] },
        {
          stageName: 'CI',
          actions: projects.map(
            ({ job, project }) =>
              new actions.CodeBuildAction({
                actionName: job.id,
                project,
                input: sourceOutput,
                outputs: [new codepipeline.Artifact(`${job.id}Output`)],
              }),
          ),
        },
      ],
    });

    if (props.prChecks) {
      const credential = new codebuild.CfnSourceCredential(this, 'GitHubCredential', {
        authType: 'CODECONNECTIONS',
        serverType: 'GITHUB',
        token: connection.attrConnectionArn,
      });
      for (const { job } of projects) {
        const pr = new codebuild.Project(this, `${job.id}PrCheck`, {
          projectName: `${name}-${job.id.toLowerCase()}-pr`,
          source: codebuild.Source.gitHub({
            owner: config.github.owner,
            repo: config.github.repo,
            reportBuildStatus: true,
            webhook: true,
            webhookFilters: [
              codebuild.FilterGroup.inEventOf(
                codebuild.EventAction.PULL_REQUEST_CREATED,
                codebuild.EventAction.PULL_REQUEST_UPDATED,
                codebuild.EventAction.PULL_REQUEST_REOPENED,
              ),
            ],
          }),
          buildSpec: codebuild.BuildSpec.fromSourceFilename(job.buildspec),
          environment: environmentFor(job),
          cache: codebuild.Cache.bucket(cache, { prefix: `${job.id.toLowerCase()}-pr` }),
          timeout: job.timeout,
        });
        pr.node.addDependency(credential);
      }
    }
  }
}
