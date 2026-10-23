package io.github.chriexpe.papo.compose;

import android.database.Cursor;
import android.database.MatrixCursor;
import android.os.CancellationSignal;
import android.os.ParcelFileDescriptor;
import android.provider.DocumentsContract.Document;
import android.provider.DocumentsContract.Root;
import android.provider.DocumentsProvider;
import java.io.File;
import java.io.FileNotFoundException;
import java.io.IOException;
import java.util.UUID;

/** Standalone provider in the test APK; its process has no app dependencies. */
public final class SettingsDocumentsProvider extends DocumentsProvider {
    private static final String[] COLUMNS = {Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME,
        Document.COLUMN_MIME_TYPE, Document.COLUMN_FLAGS, Document.COLUMN_SIZE};
    private File directory() {
        File dir = new File(getContext().getCacheDir(), "download-documents");
        dir.mkdirs();
        return dir;
    }
    private File file(String id) {
        if (!id.matches("[a-zA-Z0-9-]+\\.bin")) throw new IllegalArgumentException("Invalid fixture ID");
        return new File(directory(), id);
    }
    @Override public boolean onCreate() { return true; }
    @Override public Cursor queryRoots(String[] projection) {
        MatrixCursor cursor = new MatrixCursor(projection != null ? projection : new String[] {
            Root.COLUMN_ROOT_ID, Root.COLUMN_DOCUMENT_ID, Root.COLUMN_TITLE, Root.COLUMN_FLAGS});
        cursor.newRow().add(Root.COLUMN_ROOT_ID, "fixture").add(Root.COLUMN_DOCUMENT_ID, "root")
            .add(Root.COLUMN_TITLE, "Papo test folder").add(Root.COLUMN_FLAGS, Root.FLAG_SUPPORTS_CREATE);
        return cursor;
    }
    @Override public Cursor queryDocument(String id, String[] projection) {
        boolean root = id.equals("root");
        MatrixCursor cursor = new MatrixCursor(projection != null ? projection : COLUMNS);
        cursor.newRow().add(Document.COLUMN_DOCUMENT_ID, id)
            .add(Document.COLUMN_DISPLAY_NAME, root ? "Papo test folder" : id)
            .add(Document.COLUMN_MIME_TYPE, root ? Document.MIME_TYPE_DIR : "application/octet-stream")
            .add(Document.COLUMN_FLAGS, root ? Document.FLAG_DIR_SUPPORTS_CREATE : Document.FLAG_SUPPORTS_WRITE | Document.FLAG_SUPPORTS_DELETE)
            .add(Document.COLUMN_SIZE, root ? 0L : file(id).length());
        return cursor;
    }
    @Override public Cursor queryChildDocuments(String parent, String[] projection, String sortOrder) {
        return new MatrixCursor(projection != null ? projection : COLUMNS);
    }
    @Override public String createDocument(String parent, String mimeType, String displayName) throws FileNotFoundException {
        if (!parent.equals("root")) throw new FileNotFoundException("Invalid fixture parent");
        String id = UUID.randomUUID() + ".bin";
        try { if (!file(id).createNewFile()) throw new IOException("Already exists"); }
        catch (IOException error) { throw new FileNotFoundException(error.getMessage()); }
        return id;
    }
    @Override public ParcelFileDescriptor openDocument(String id, String mode, CancellationSignal signal) throws FileNotFoundException {
        return ParcelFileDescriptor.open(file(id), ParcelFileDescriptor.parseMode(mode));
    }
    @Override public void deleteDocument(String id) throws FileNotFoundException {
        if (!file(id).delete()) throw new FileNotFoundException("Couldn’t delete fixture");
    }
    @Override public boolean isChildDocument(String parent, String id) { return parent.equals("root") && !id.equals("root"); }
}
