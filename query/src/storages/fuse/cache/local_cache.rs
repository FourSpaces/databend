// Copyright 2021 Datafuse Labs.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::ffi::OsString;
use std::sync::Arc;

use async_trait::async_trait;
use common_base::tokio::sync::RwLock;
use common_cache::BytesMeter;
use common_cache::Cache;
use common_cache::DefaultHashBuilder;
use common_cache::LruCache;
use common_cache::LruDiskCache;
use common_dal::DataAccessor;
use common_exception::Result;

use crate::storages::cache::StorageCache;
use crate::storages::fuse::cache::metrics::CacheDeferMetrics;

pub struct LocalCacheConfig {
    pub memory_cache_size_mb: u64,
    pub disk_cache_size_mb: u64,
    pub disk_cache_root: String,
    pub tenant_id: String,
    pub cluster_id: String,
}

type MemCache = Arc<RwLock<LruCache<OsString, Vec<u8>, DefaultHashBuilder, BytesMeter>>>;

// TODO maybe distinct segments cache and snapshots cache
#[derive(Clone, Debug)]
pub struct LocalCache {
    pub disk_cache: Arc<RwLock<LruDiskCache>>,
    pub mem_cache: MemCache,
    tenant_id: String,
    cluster_id: String,
}

impl LocalCache {
    pub fn create(conf: LocalCacheConfig) -> Result<Box<dyn StorageCache>> {
        let disk_cache = Arc::new(RwLock::new(LruDiskCache::new(
            conf.disk_cache_root,
            conf.disk_cache_size_mb * 1024 * 1024,
        )?));
        Ok(Box::new(LocalCache {
            mem_cache: Arc::new(RwLock::new(LruCache::with_meter(
                conf.memory_cache_size_mb * 1024 * 1024,
                BytesMeter,
            ))),
            disk_cache,
            tenant_id: conf.tenant_id,
            cluster_id: conf.cluster_id,
        }))
    }

    async fn get_from_mem_cache(&self, location: &str, da: &dyn DataAccessor) -> Result<Vec<u8>> {
        let loc: OsString = location.to_owned().into();
        let mut metrics = CacheDeferMetrics {
            tenant_id: self.tenant_id.as_str(),
            cluster_id: self.cluster_id.as_str(),
            cache_hit: false,
            read_bytes: 0,
        };

        // get data from memory cache
        let mut mem_cache = self.mem_cache.write().await;
        if let Some(data) = mem_cache.get(&loc) {
            metrics.cache_hit = true;
            metrics.read_bytes = data.len() as u64;

            return Ok(data.clone());
        }
        let data = da.read(location).await?;
        mem_cache.put(loc, data.clone());
        metrics.read_bytes = data.len() as u64;
        Ok(data)
    }
}

#[async_trait]
impl StorageCache for LocalCache {
    async fn get(&self, location: &str, da: &dyn DataAccessor) -> Result<Vec<u8>> {
        self.get_from_mem_cache(location, da).await
    }
}
