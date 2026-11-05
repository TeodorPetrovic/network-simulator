export type ResourceMonitor = {
  sampled_at_unix_ms: number | null;
  host: {
    cpu_usage_percent: number | null;
    cpu_count: number | null;
    memory_used_bytes: number | null;
    memory_total_bytes: number | null;
    load_average: number[] | null;
  };
  controller: {
    memory_rss_bytes: number | null;
    thread_count: number | null;
  };
};

export type IshareImage = {
  id: number;
  name: string;
  image_type: string;
  size: string;
  total_size: number;
  file_count: number;
  files?: string[];
  catalog: string;
};

export type IshareDownloadResult = {
  id: number;
  name: string;
  image_type: string;
  files: string[];
  source_files?: string[];
  image_directory: string;
};

