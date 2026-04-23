import { Component, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { ActivatedRoute, Router, RouterModule } from '@angular/router';
import { DocsOverviewComponent } from './overview/overview.component';

// Import available doc page components
import { ProfilingGuideComponent } from './pages/profiling-guide/profiling-guide.component';
import { FrameworkComparisonComponent } from './pages/framework-comparison/framework-comparison.component';

interface DocMetadata {
  id: string;
  title: string;
  category: string;
  component?: any;
  hasComponent?: boolean;
}

@Component({
  selector: 'app-docs',
  standalone: true,
  imports: [
    CommonModule,
    RouterModule,
    DocsOverviewComponent,
    ProfilingGuideComponent,
    FrameworkComparisonComponent,
  ],
  templateUrl: './docs.component.html',
  styleUrls: ['./docs.component.scss'],
})
export class DocsComponent implements OnInit {
  loading = signal(true);
  error = signal<string | null>(null);
  currentDoc = signal<DocMetadata | null>(null);
  activeComponent = signal<string>('overview');

  // Documentation metadata with user-friendly titles
  docs: DocMetadata[] = [
    // 1. Getting Started - First things first
    { id: 'readme', title: 'Overview', category: 'Getting Started', hasComponent: true },
    { id: 'micro-framework-guide', title: 'Micro-Framework Mode', category: 'Getting Started' },
    { id: 'project-templates', title: 'Project Templates', category: 'Getting Started' },
    { id: 'config-guide', title: 'Configuration', category: 'Getting Started' },
    { id: 'macro-overview', title: 'Macros Overview', category: 'Getting Started' },

    // 2. Core Concepts - Foundation before building
    { id: 'di-guide', title: 'Dependency Injection', category: 'Core Concepts' },
    { id: 'lifecycle-hooks', title: 'Lifecycle Hooks', category: 'Core Concepts' },

    // 3. Routing & Controllers - Learn to handle requests first
    { id: 'route-groups', title: 'Route Groups', category: 'Routing' },
    { id: 'route-constraints', title: 'Route Constraints', category: 'Routing' },
    { id: 'request-extractors', title: 'Request Extractors', category: 'Routing' },
    { id: 'use-middleware', title: 'Middleware', category: 'Routing' },
    { id: 'guards-interceptors', title: 'Guards & Interceptors', category: 'Routing' },
    { id: 'use-guard', title: 'Using Guards', category: 'Routing' },

    // 4. Request & Response - HTTP handling
    { id: 'http-status-errors', title: 'HTTP Errors', category: 'HTTP' },
    { id: 'request-timeouts', title: 'Timeouts', category: 'HTTP' },
    { id: 'streaming-responses', title: 'Streaming', category: 'HTTP' },
    { id: 'compression', title: 'Compression', category: 'HTTP' },
    { id: 'https-guide', title: 'HTTPS & TLS', category: 'HTTP' },
    { id: 'acme-certificates', title: 'SSL Certificates', category: 'HTTP' },

    // 5. Security - After understanding routing
    { id: 'auth-guide', title: 'Authentication', category: 'Security' },
    { id: 'oauth2-providers', title: 'OAuth2 & Social Login', category: 'Security' },
    { id: 'session-guide', title: 'Sessions', category: 'Security' },
    { id: 'rate-limiting', title: 'Rate Limiting', category: 'Security' },
    { id: 'security-guide', title: 'Security Best Practices', category: 'Security' },
    { id: 'security-advanced', title: 'Advanced Security', category: 'Security' },

    // 6. API Features - Building APIs
    { id: 'api-versioning', title: 'API Versioning', category: 'API Features' },
    { id: 'pagination-filtering', title: 'Pagination & Filtering', category: 'API Features' },
    { id: 'content-negotiation', title: 'Content Negotiation', category: 'API Features' },
    { id: 'response-caching', title: 'HTTP Caching', category: 'API Features' },
    { id: 'etag-conditional', title: 'ETags', category: 'API Features' },

    // 7. Data & Caching - Persistence layer
    { id: 'cache-improvements', title: 'Caching Strategies', category: 'Data & Caching' },
    { id: 'redis-guide', title: 'Redis', category: 'Data & Caching' },

    // 8. Background Processing - Async work
    { id: 'queue-guide', title: 'Job Queues', category: 'Background Jobs' },
    { id: 'cron-guide', title: 'Scheduled Tasks', category: 'Background Jobs' },
    { id: 'graceful-shutdown', title: 'Graceful Shutdown', category: 'Background Jobs' },

    // 9. Real-Time - Live communication
    { id: 'websocket-sse', title: 'WebSockets & SSE', category: 'Real-Time' },
    { id: 'webhooks', title: 'Webhooks', category: 'Real-Time' },

    // 10. GraphQL - Alternative API paradigm
    { id: 'graphql-guide', title: 'GraphQL', category: 'GraphQL' },
    { id: 'graphql-config', title: 'GraphQL Config', category: 'GraphQL' },

    // 11. Cloud & Deployment - Production readiness
    { id: 'cloud-providers', title: 'Cloud SDKs', category: 'Cloud & Deployment' },
    { id: 'deployment-guide', title: 'Deployment Guide', category: 'Cloud & Deployment' },
    { id: 'docker-guide', title: 'Docker', category: 'Cloud & Deployment' },
    { id: 'kubernetes-guide', title: 'Kubernetes', category: 'Cloud & Deployment' },
    { id: 'scaling-guide', title: 'Scaling', category: 'Cloud & Deployment' },

    // 12. Observability - Monitoring and debugging
    { id: 'logging-guide', title: 'Logging', category: 'Observability' },
    { id: 'debug-logging', title: 'Debug Logging', category: 'Observability' },
    { id: 'health-check', title: 'Health Checks', category: 'Observability' },
    { id: 'metrics-guide', title: 'Metrics', category: 'Observability' },
    { id: 'grafana-dashboards', title: 'Grafana Dashboards', category: 'Observability' },
    { id: 'opentelemetry-guide', title: 'OpenTelemetry', category: 'Observability' },
    { id: 'error-correlation', title: 'Error Tracking', category: 'Observability' },
    { id: 'audit-guide', title: 'Audit Logging', category: 'Observability' },

    // 13. Testing - Quality assurance
    { id: 'testing-guide', title: 'Testing', category: 'Testing' },
    { id: 'testing-coverage', title: 'Coverage', category: 'Testing' },

    // 14. Performance - Optimization tools
    { id: 'profiling-guide', title: 'CPU Profiling', category: 'Performance', hasComponent: true },

    // 15. Benchmarks - Performance comparisons
    { id: 'framework-comparison', title: 'vs Actix vs Axum', category: 'Benchmarks', hasComponent: true },
    { id: 'armature-vs-nodejs', title: 'vs Node.js', category: 'Benchmarks' },
    { id: 'armature-vs-nextjs', title: 'vs Next.js', category: 'Benchmarks' },
  ];

  // Group docs by category
  docsByCategory = signal<{ [key: string]: DocMetadata[] }>({});

  constructor(
    private route: ActivatedRoute,
    private router: Router
  ) {
    // Group docs by category
    const grouped: { [key: string]: DocMetadata[] } = {};
    this.docs.forEach((doc) => {
      if (!grouped[doc.category]) {
        grouped[doc.category] = [];
      }
      grouped[doc.category].push(doc);
    });
    this.docsByCategory.set(grouped);
  }

  ngOnInit() {
    this.route.paramMap.subscribe((params) => {
      const docId = params.get('id');
      if (docId) {
        this.loadDoc(docId);
      } else {
        // Default to README
        this.router.navigate(['/docs/readme']);
      }
    });
  }

  loadDoc(docId: string) {
    this.loading.set(true);
    this.error.set(null);

    const doc = this.docs.find((d) => d.id === docId);
    if (!doc) {
      this.error.set('Documentation not found');
      this.loading.set(false);
      return;
    }

    this.currentDoc.set(doc);
    this.activeComponent.set(docId);
    this.loading.set(false);

    // Scroll to top on navigation
    window.scrollTo({ top: 0, behavior: 'instant' });
  }

  getCategories(): string[] {
    return Object.keys(this.docsByCategory());
  }

  isActiveDoc(docId: string): boolean {
    return this.currentDoc()?.id === docId;
  }
}
