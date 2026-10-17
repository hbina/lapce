use super::model::{FileNodeItem, FileNodeViewData};
use floem::views::VirtualVector;

pub struct FileNodeVirtualList {
    file_node_item: FileNodeItem,
}

impl FileNodeVirtualList {
    pub fn new(file_node_item: FileNodeItem) -> Self {
        Self { file_node_item }
    }
}

impl VirtualVector<FileNodeViewData> for FileNodeVirtualList {
    fn total_len(&self) -> usize {
        self.file_node_item.children_open_count + 1
    }

    fn slice(
        &mut self,
        range: std::ops::Range<usize>,
    ) -> impl Iterator<Item = FileNodeViewData> {
        let root = &self.file_node_item;

        let min = range.start;
        let max = range.end;
        let mut view_items = Vec::new();

        root.append_view_slice(&mut view_items, min, max, 0, 1);

        view_items.into_iter()
    }
}
